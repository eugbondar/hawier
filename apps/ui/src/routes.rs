use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{Form, Path as AxumPath, Query, State},
    http::{header::SET_COOKIE, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{any, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    error::AppError,
    hawkbit, help, html, logs,
    logs::ActionLogDraft,
    resources::{self, Group, Resource},
    session::{self, LoginRequest, SessionResponse},
    state::AppState,
    view::{self, Cell, DetailExtra, PickerSpec},
    views::{self, SavedViewDraft},
};

/// This build of the console: `vYY.MM.DD-<short sha>` (see build.rs).
pub const BUILD: &str = env!("HAWIER_BUILD");

#[derive(Deserialize, Default)]
pub struct ListQuery {
    pub view: Option<String>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub columns: Option<String>,
    pub page: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
struct LoginPageQuery {
    error: Option<String>,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

#[derive(Deserialize)]
struct MoveViewForm {
    direction: String,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    build: &'static str,
    hawkbit_mgmt_url: String,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(overview))
        .route("/login", get(login_page).post(login_form))
        .route("/logout", post(logout_form))
        .route("/search/live", get(live_search))
        .route("/search", get(search_page))
        .route("/search/history/{id}/delete", post(delete_search_history))
        .route("/search/history/clear", post(clear_search_history))
        // Typeahead behind every [data-picker]: no dropdown is ever filled
        // with a whole server-side list.
        .route("/pick/{resource}", get(pick_options))
        // The manual. `/help/panel` is a static sibling of the wildcard, so it
        // wins the match the same way `/{resource}/new` wins over `/{id}`.
        .route("/help", get(help_index))
        .route("/help/panel", get(help_panel))
        .route("/help/{*topic}", get(help_topic_page))
        // Both pages became ordinary resources; keep old links alive.
        .route(
            "/errors",
            get(|| async { Redirect::to("/incidents").into_response() }),
        )
        .route(
            "/logs",
            get(|| async { Redirect::to("/action-log").into_response() }),
        )
        .route(
            "/software-modules/{id}/artifacts",
            post(upload_artifact_route)
                // Artifact uploads must not be capped by axum's 2 MB default.
                .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 1024)),
        )
        .route(
            "/software-modules/{id}/artifacts/{artifact_id}/delete",
            post(delete_artifact_route),
        )
        // The mgmt endpoint needs HTTP Basic, which the browser does not have:
        // the console streams the bytes through under the session instead.
        .route(
            "/software-modules/{id}/artifacts/{artifact_id}/download",
            get(download_artifact),
        )
        .route("/distribution-sets/{id}/modules", post(attach_module))
        .route(
            "/distribution-sets/{id}/modules/{sm_id}/detach",
            post(detach_module),
        )
        .route("/targets/{id}/assign", post(assign_ds_to_target))
        .route("/targets/{id}/tags", post(add_target_tag))
        .route(
            "/targets/{id}/tags/{tag_id}/remove",
            post(remove_target_tag),
        )
        .route(
            "/targets/{id}/auto-confirm/{toggle}",
            post(toggle_auto_confirm),
        )
        .route("/targets/{id}/type", post(set_target_type))
        .route("/targets/{id}/group", post(set_target_group))
        .route("/targets/{id}/group/clear", post(clear_target_group))
        .route("/target-types/{id}/ds-types", post(add_compatible_ds_type))
        .route(
            "/target-types/{id}/ds-types/{ds_type_id}/remove",
            post(remove_compatible_ds_type),
        )
        .route("/target-filters/{id}/auto-assign", post(set_auto_assign))
        .route(
            "/target-filters/{id}/auto-assign/clear",
            post(clear_auto_assign),
        )
        // Metadata lives on three resources; the handler keeps the set closed.
        .route("/{resource}/{id}/metadata", post(save_metadata))
        .route("/{resource}/{id}/metadata/delete", post(delete_metadata))
        .route("/targets/{id}/actions/{action_id}", get(action_page))
        .route(
            "/targets/{id}/actions/{action_id}/cancel",
            post(cancel_target_action),
        )
        .route("/distribution-sets/{id}/tags", post(add_ds_tag))
        .route(
            "/distribution-sets/{id}/tags/{tag_id}/remove",
            post(remove_ds_tag),
        )
        .route(
            "/distribution-set-types/{id}/modules",
            post(add_ds_type_module),
        )
        .route(
            "/distribution-set-types/{id}/modules/{kind}/{sm_type_id}/remove",
            post(remove_ds_type_module),
        )
        .route("/rollouts/{id}/groups/{group_id}", get(rollout_group_page))
        .route("/settings", get(settings_page))
        .route("/settings/config", post(update_tenant_config))
        .route("/settings/config/delete", post(delete_tenant_config))
        .route("/settings/extensions", post(update_ext_param))
        .route("/settings/extensions/delete", post(delete_ext_param))
        .route("/views", post(save_view))
        .route("/views/{id}/edit", get(edit_view_form).post(update_view))
        .route("/views/{id}/delete", post(delete_view))
        .route("/views/{id}/duplicate", post(duplicate_view))
        .route("/views/{id}/move", post(move_view))
        .route("/assets/{*path}", get(static_asset))
        .route("/api/health", get(health))
        .route("/api/session", get(api_session))
        .route("/api/login", post(api_login))
        .route("/api/logout", post(api_logout))
        .route("/api/hawkbit/status", get(api_hawkbit_status))
        .route("/api/hawkbit/summary", get(api_summary))
        .route("/api/hawkbit/rest/v1/{*path}", any(proxy_hawkbit))
        .route("/{resource}", get(list_page))
        .route(
            "/{resource}/new",
            get(new_resource_form).post(create_resource),
        )
        .route("/{resource}/{id}", get(detail_page))
        .route(
            "/{resource}/{id}/edit",
            get(edit_resource_form).post(update_resource),
        )
        .route("/{resource}/bulk", post(bulk_action))
        .route("/{resource}/{id}/delete", post(delete_resource))
        .route("/{resource}/{id}/actions/{action}", post(resource_action))
        .fallback(get(not_found))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Navigation column and topbar state. Saved views are fetched once for the
/// whole tree — one round-trip, not one per resource.
async fn layout(
    state: &AppState,
    session: Option<&session::Session>,
    title: &str,
    active: Option<&Resource>,
    active_view: Option<&str>,
    search_query: Option<&str>,
    // The manual section this page answers with. Empty means "the chapter of
    // the list being shown" — every page must name one, which is what stops a
    // new page from shipping undocumented.
    help: &str,
) -> view::Layout {
    let mut saved = match session {
        Some(session) => views::all_for_user(state, session).await,
        None => Vec::new(),
    };
    saved.sort_by_key(|item| item.position);

    let groups = [
        Group::Inventory,
        Group::Deployment,
        Group::Repository,
        Group::Admin,
    ]
    .into_iter()
    .map(|group| {
        let group_active = active.is_some_and(|resource| resource.group == group);
        let resources_in_group = resources::all()
            .into_iter()
            .filter(|resource| resource.group == group)
            .map(|resource| {
                let is_active = active.is_some_and(|item| item.id == resource.id);
                view::NavResource {
                    help: help::page_key(Some(&resource.id)),
                    id: resource.id.clone(),
                    label: resource.label.clone(),
                    icon: view::resource_icon(&resource),
                    active: is_active,
                    views: resources::system_views(&resource.id)
                        .iter()
                        .map(|item| view::NavView {
                            help: help::view_key(&resource.id, &item.id),
                            href: format!(
                                "/{}?view={}",
                                resource.id,
                                html::encode_component(&item.id)
                            ),
                            edit_href: String::new(),
                            label: item.label.clone(),
                            active: is_active && active_view == Some(item.id.as_str()),
                        })
                        .collect(),
                    saved: saved
                        .iter()
                        .filter(|item| item.resource == resource.id)
                        .map(|item| view::NavView {
                            help: "console/views#saved-views".to_string(),
                            href: format!(
                                "/{}?view={}",
                                resource.id,
                                html::encode_component(&item.id)
                            ),
                            edit_href: format!("/views/{}/edit", html::encode_path(&item.id)),
                            label: item.name.clone(),
                            active: is_active && active_view == Some(item.id.as_str()),
                        })
                        .collect(),
                }
            })
            .collect();
        view::NavGroup {
            help: "console/lists#the-list-page",
            key: view::group_key(group),
            label: view::group_label(group),
            open: group_active || group != Group::Admin,
            active: group_active,
            is_admin: group == Group::Admin,
            resources: resources_in_group,
        }
    })
    .collect();

    view::Layout {
        title: title.to_string(),
        help: if help.is_empty() {
            help::page_key(active.map(|resource| resource.id.as_str()))
        } else {
            help.to_string()
        },
        search_query: search_query.unwrap_or_default().to_string(),
        groups,
        session: session.map(|session| view::Chip {
            tenant: session.tenant.clone(),
            username: session.username.clone(),
        }),
    }
}

#[derive(Deserialize, Default)]
struct HelpQuery {
    q: Option<String>,
}

#[derive(Deserialize, Default)]
struct HelpKeyQuery {
    key: Option<String>,
}

/// The table of contents, or the result of searching it.
async fn help_index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HelpQuery>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let q = query.q.unwrap_or_default();
    let hits = help::search(&q)
        .into_iter()
        .map(|(topic, section)| view::HelpHit {
            href: help::article_href(&section.key),
            title: section.title.as_str(),
            topic_title: topic.title.as_str(),
            summary: section.summary_line(),
        })
        .collect();
    view::respond(&view::HelpIndexPage {
        layout: layout(
            &state,
            Some(&session),
            "Help",
            None,
            None,
            None,
            "console/help#the-manual",
        )
        .await,
        q,
        areas: view::help_areas(),
        hits,
    })
}

/// One chapter, every section of it.
async fn help_topic_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(topic_id): AxumPath<String>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let Some(topic) = help::topic(topic_id.trim_end_matches('/')) else {
        return not_found().await;
    };
    view::respond(&view::HelpTopicPage {
        layout: layout(
            &state,
            Some(&session),
            &topic.title,
            None,
            None,
            None,
            topic.id,
        )
        .await,
        topic_id: topic.id,
        title: topic.title.as_str(),
        intro: &topic.intro,
        sections: topic
            .sections
            .iter()
            .map(|section| view::HelpSectionView {
                status: section.status,
                anchor: section.anchor.as_str(),
                title: section.title.as_str(),
                body: &section.body,
            })
            .collect(),
    })
}

/// The What's-This panel. A key nobody has written yet is not an error: the
/// fragment says so and offers the manual.
async fn help_panel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HelpKeyQuery>,
) -> Response {
    if session::current(&state, &headers).await.is_none() {
        return (StatusCode::UNAUTHORIZED, Html("Login required".to_string())).into_response();
    }
    let key = query.key.unwrap_or_default();
    match help::resolve(&key) {
        Some((topic, Some(section))) => view::respond(&view::HelpPanelFragment {
            found: true,
            topic_title: topic.title.as_str(),
            title: section.title.as_str(),
            summary: &section.summary,
            article_href: help::article_href(&section.key),
        }),
        Some((topic, None)) => view::respond(&view::HelpPanelFragment {
            found: true,
            topic_title: topic.title.as_str(),
            title: topic.title.as_str(),
            summary: &topic.intro,
            article_href: format!("/help/{}", topic.id),
        }),
        None => view::respond(&view::HelpPanelFragment {
            found: false,
            topic_title: "Help",
            title: "",
            summary: &[],
            article_href: "/help".to_string(),
        }),
    }
}

/// A page that says one thing: an upstream failure, or the result of a bulk
/// operation. The frame is never stripped — navigation stays usable.
async fn message_page(
    state: &AppState,
    session: &session::Session,
    title: &str,
    active: Option<&Resource>,
    message: String,
    danger: bool,
) -> Response {
    view::respond(&view::MessagePage {
        layout: layout(state, Some(session), title, active, None, None, "").await,
        message,
        danger,
    })
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

/// Incidents worth a dashboard slot: device/API traffic, or any server fault.
const IMPORTANT_INCIDENTS: &str = "kind=in=(ddi,mgmt),status=ge=500";

/// Update statuses shown as cards on the overview (hawkBit's TargetUpdateStatus).
const TARGET_STATUS_CARDS: [&str; 5] = ["in_sync", "pending", "error", "registered", "unknown"];

async fn overview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    // Every counter below is one upstream round-trip; run them concurrently.
    let count_of = |value: Value| value.get("count").and_then(Value::as_u64);
    // Built once and reused: `all()` merges the built-in table with whatever
    // extensions declared, so calling it per use would rebuild that each time.
    let dashboard_resources = resources::all();
    let resource_counts = futures::future::join_all(
        dashboard_resources
            .iter()
            .map(|resource| hawkbit::count_resource(&state, &session, &resource.endpoint, None)),
    );
    let status_queries: Vec<String> = TARGET_STATUS_CARDS
        .iter()
        .map(|status| format!("updatestatus=={status}"))
        .collect();
    let status_totals = futures::future::join_all(
        status_queries
            .iter()
            .map(|q| hawkbit::count_resource(&state, &session, "targets", Some(q))),
    );
    let (resource_counts, status_totals) = tokio::join!(resource_counts, status_totals);

    let counts: Vec<view::CountRow> = dashboard_resources
        .into_iter()
        .zip(resource_counts.into_iter().map(count_of))
        .map(|(resource, count)| view::CountRow {
            id: resource.id.clone(),
            label: resource.label.clone(),
            count: view::count_text(count),
            group: view::group_label(resource.group),
            new_href: resource
                .create_verified
                .then(|| format!("/{}/new", resource.id)),
        })
        .collect();
    let status_cards: Vec<view::StatCard> = TARGET_STATUS_CARDS
        .iter()
        .zip(status_totals.into_iter().map(count_of))
        .map(|(status, count)| view::status_card(status, count))
        .collect();

    let active_rollouts = match resources::by_id("rollouts") {
        Some(rollouts) => hawkbit::fetch_page(
            &state,
            &session,
            &rollouts.endpoint,
            Some("status=in=(running,paused,starting,ready)"),
            Some("id:DESC"),
            1,
            10,
        )
        .await
        .map(|value| hawkbit::read_items(&value))
        .unwrap_or_default(),
        None => Vec::new(),
    };
    // Only what matters on a dashboard: devices, API clients, server faults.
    // Console-side noise (our own bad request) is counted, not listed.
    let recent_errors = match resources::by_id("incidents") {
        Some(resource) => hawkbit::fetch_page(
            &state,
            &session,
            &resource.endpoint,
            Some(IMPORTANT_INCIDENTS),
            Some("incident:DESC"),
            1,
            5,
        )
        .await
        .map(|value| hawkbit::read_items(&value))
        .unwrap_or_default(),
        None => Vec::new(),
    };
    let console_noise = hawkbit::count_resource(
        &state,
        &session,
        "incidents",
        Some("kind=in=(console,ui);status=lt=500"),
    )
    .await
    .get("count")
    .and_then(Value::as_u64)
    .unwrap_or(0);

    view::respond(&view::OverviewPage {
        layout: layout(
            &state,
            Some(&session),
            "Overview",
            None,
            None,
            None,
            "start/console",
        )
        .await,
        status_cards,
        counts,
        rollouts: active_rollouts
            .iter()
            .map(|rollout| view::RolloutSummary {
                href: format!(
                    "/rollouts/{}",
                    html::encode_path(&view::id_of(rollout, "id"))
                ),
                name: view::str_or(rollout, "name", "-"),
                status: view::str_or(rollout, "status", "-"),
                finished: rollout
                    .pointer("/totalTargetsPerStatus/finished")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                total: rollout
                    .get("totalTargets")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            })
            .collect(),
        incidents: recent_errors
            .iter()
            .map(|error| view::IncidentSummary {
                number: view::text_of(error, "incident"),
                status: view::id_of(error, "status"),
                route: format!(
                    "{} {}",
                    view::text_of(error, "method"),
                    view::text_of(error, "path")
                ),
                message: view::text_of(error, "message")
                    .chars()
                    .take(80)
                    .collect::<String>(),
            })
            .collect(),
        console_noise,
    })
}

async fn login_page(
    State(state): State<AppState>,
    Query(query): Query<LoginPageQuery>,
    headers: HeaderMap,
) -> Response {
    if session::current(&state, &headers).await.is_some() {
        return Redirect::to("/").into_response();
    }
    view::respond(&view::LoginPage { error: query.error })
}

async fn login_form(State(state): State<AppState>, Form(request): Form<LoginRequest>) -> Response {
    match session::authenticate(&state, request).await {
        Ok((_, cookie)) => {
            let mut response = Redirect::to("/").into_response();
            response
                .headers_mut()
                .insert(SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
            response
        }
        Err(error) => Redirect::to(&format!(
            "/login?error={}",
            html::encode_component(&error.message)
        ))
        .into_response(),
    }
}

async fn logout_form(State(state): State<AppState>, headers: HeaderMap) -> Response {
    session::clear(&state, &headers, Redirect::to("/login").into_response()).await
}

async fn settings_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let (configs, server, extensions) = tokio::join!(
        hawkbit::get_json(&state, &session, "system/configs"),
        state.ui_api.get_json::<Value>("/ui/v1/version"),
        hawkbit::get_json(&state, &session, "system/extensions"),
    );
    let configs = configs.unwrap_or_else(|_| json!({}));
    let server = server.unwrap_or_else(|_| json!({}));
    // Three ways this fails and all of them keep the page: the node is down,
    // the operator lacks READ_SYSTEM, or stock hawkBit is behind the mgmt URL
    // and has never heard of the resource. The panels say so in their own
    // data area; the frame does not depend on it (UI/UX rule 2).
    let extensions = match extensions {
        Ok(value) => view::extensions_panel(&value),
        Err(error) => view::ExtensionsPanel::unavailable(&error.message),
    };
    let mut rows: Vec<view::ConfigRow> = configs
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(key, entry)| view::ConfigRow {
                    key: key.clone(),
                    value: entry
                        .get("value")
                        .map(html::render_plain_value)
                        .unwrap_or_default(),
                    global: entry
                        .get("global")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| a.key.cmp(&b.key));

    view::respond(&view::SettingsPage {
        layout: layout(
            &state,
            Some(&session),
            "System Settings",
            None,
            None,
            None,
            "console/settings",
        )
        .await,
        tenant: session.tenant.clone(),
        username: session.username.clone(),
        mgmt_url: state.config.hawkbit_mgmt_url.clone(),
        state_api: state.config.hawier_api_url.clone(),
        console_build: BUILD,
        server_build: view::str_or(&server, "build", "unavailable"),
        server_node: view::text_of(&server, "node"),
        server_role: view::text_of(&server, "role"),
        // Only worth saying when it is not the ordinary answer.
        console_profile: match cfg!(debug_assertions) {
            true => "debug".to_string(),
            false => String::new(),
        },
        server_profile: match view::text_of(&server, "profile").as_str() {
            "release" | "" => String::new(),
            other => other.to_string(),
        },
        configs: rows,
        extensions,
    })
}

async fn list_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(resource_id): AxumPath<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };

    let default_view = resources::default_view_id(&resource);
    let view_id = query.view.as_deref().unwrap_or(&default_view);
    let system_view = resources::system_views(&resource.id)
        .into_iter()
        .find(|candidate| candidate.id == view_id);
    let saved_view = views::find_for_resource(&state, &session, &resource.id, view_id).await;
    let q = query
        .q
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            saved_view
                .as_ref()
                .map(|view| view.q.clone())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| system_view.as_ref().and_then(|view| view.q.clone()));
    let sort = query
        .sort
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            saved_view
                .as_ref()
                .map(|view| view.sort.clone())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| system_view.as_ref().and_then(|view| view.sort.clone()))
        .or_else(|| resource.default_sort.clone());
    let page = query.page.unwrap_or(1).max(1);
    let limit = query
        .limit
        .or_else(|| saved_view.as_ref().map(|view| view.page_size))
        .unwrap_or(25)
        .clamp(10, 100);
    let columns_csv = query
        .columns
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            saved_view
                .as_ref()
                .map(|view| view.columns.clone())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            system_view
                .and_then(|view| view.columns)
                .map(|columns| columns.join(","))
        })
        .unwrap_or_else(|| resource.columns.join(","));
    let columns = resources::selected_columns(&resource, &columns_csv);

    // Success and failure render the same frame — only the data area differs.
    let (items, total, error) = match hawkbit::fetch_page(
        &state,
        &session,
        &resource.endpoint,
        q.as_deref(),
        sort.as_deref(),
        page,
        limit,
    )
    .await
    {
        Ok(page_value) => {
            let items = hawkbit::read_items(&page_value);
            let total = hawkbit::read_total(&page_value).unwrap_or(items.len());
            (items, total, None)
        }
        Err(error) => (Vec::new(), 0, Some(error.message)),
    };

    let mut pager = view::Pager::new(page, limit, total, items.len(), &|target| {
        html::list_href(&resource, &query, None, None, Some(target))
    });
    pager.jump = Some(view::PagerJump {
        action: format!("/{}", resource.id),
        fields: pager_fields(view_id, q.as_deref(), sort.as_deref(), limit, &columns),
    });

    view::respond(&view::ListPage {
        layout: layout(
            &state,
            Some(&session),
            &resource.label,
            Some(&resource),
            Some(view_id),
            None,
            "",
        )
        .await,
        resource_id: resource.id.clone(),
        label: resource.label.clone(),
        new_href: resource
            .create_verified
            .then(|| format!("/{}/new", resource.id)),
        view: view_id.to_string(),
        q: q.clone().unwrap_or_default(),
        sort: sort.clone().unwrap_or_default(),
        limit,
        columns_csv: columns.join(","),
        column_picks: resource
            .columns
            .iter()
            .map(|column| view::ColumnPick {
                name: (*column).to_string(),
                checked: columns.iter().any(|selected| selected == column),
            })
            .collect(),
        headers: columns
            .iter()
            .map(|column| view::Header {
                help: help::field_key(&resource.id, column),
                label: column.clone(),
                href: html::list_href(
                    &resource,
                    &query,
                    Some(("sort", &html::next_sort(column, sort.as_deref()))),
                    None,
                    Some(page),
                ),
            })
            .collect(),
        rows: items
            .iter()
            .map(|item| {
                let id = html::row_id(&resource, item);
                view::Row {
                    href: html::row_href(&resource, item),
                    id,
                    cells: columns
                        .iter()
                        .map(|column| view::cell_at(item, column))
                        .collect(),
                }
            })
            .collect(),
        colspan: columns.len() + 1,
        pager,
        bulk: bulk_groups(resource),
        error,
    })
}

/// Carry the current view/filter/sort/limit through the page-jump form.
fn pager_fields(
    view_id: &str,
    q: Option<&str>,
    sort: Option<&str>,
    limit: usize,
    columns: &[String],
) -> Vec<view::Hidden> {
    let mut fields = vec![view::Hidden {
        name: "view".to_string(),
        value: view_id.to_string(),
    }];
    for (name, value) in [("q", q.unwrap_or("")), ("sort", sort.unwrap_or(""))] {
        if !value.is_empty() {
            fields.push(view::Hidden {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }
    fields.push(view::Hidden {
        name: "limit".to_string(),
        value: limit.to_string(),
    });
    fields.push(view::Hidden {
        name: "columns".to_string(),
        value: columns.join(","),
    });
    fields
}

fn bulk_picker(kind: resources::BulkPicker) -> PickerSpec {
    match kind {
        resources::BulkPicker::DistributionSet => {
            PickerSpec::new("ds_id", "distribution-sets", "Find a distribution set…")
        }
        resources::BulkPicker::TargetTag => PickerSpec::new("tag_id", "target-tags", "Find a tag…"),
        resources::BulkPicker::DsTag => {
            PickerSpec::new("tag_id", "distribution-set-tags", "Find a tag…")
        }
        resources::BulkPicker::TargetGroup => {
            PickerSpec::new("group", "target-groups", "Find or type a path…").free_text()
        }
    }
    .in_form("bulk-form")
}

/// Menu entries for the selection, keeping the declaration order. Actions
/// that share a picker share one input, so `tag`/`untag` stay unambiguous.
fn bulk_groups(resource: Resource) -> Vec<view::BulkGroup> {
    let mut groups: Vec<view::BulkGroup> = Vec::new();
    let mut current: Option<Option<resources::BulkPicker>> = None;
    for action in resources::bulk_actions(&resource.id) {
        if current != Some(action.picker) {
            groups.push(view::BulkGroup {
                picker: action.picker.map(bulk_picker),
                actions: Vec::new(),
            });
            current = Some(action.picker);
        }
        if let Some(group) = groups.last_mut() {
            group.actions.push(view::BulkItem {
                id: action.id,
                label: action.label,
                danger: action.danger,
                help: help::bulk_key(action.id),
            });
        }
    }
    groups
}

/// Apply one operation to every selected row. Upstream endpoints that take
/// a list are called once; the rest are looped, and partial failure is
/// reported rather than hidden.
async fn bulk_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(resource_id): AxumPath<String>,
    Form(payload): Form<Vec<(String, String)>>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let field = |name: &str| {
        payload
            .iter()
            .find(|(key, value)| key == name && !value.is_empty())
            .map(|(_, value)| value.clone())
    };
    let ids: Vec<String> = payload
        .iter()
        .filter(|(key, _)| key == "ids")
        .map(|(_, value)| value.clone())
        .collect();
    let action = field("action").unwrap_or_default();
    let back = format!("/{}", resource.id);
    if ids.is_empty() || action.is_empty() {
        return Redirect::to(&back).into_response();
    }

    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: &resource.id,
            resource_id: "bulk",
            action: &action,
            request_summary: format!("{action} on {} selected", ids.len()),
        },
    )
    .await
    .ok();

    let outcome = run_bulk(&state, &session, &resource, &action, &ids, &field).await;
    match &outcome {
        Ok(done) => {
            finish_log(
                &state,
                log.as_ref(),
                "success",
                &format!("{done} of {}", ids.len()),
                "",
            )
            .await
        }
        Err(error) => finish_log(&state, log.as_ref(), "error", "", error).await,
    }

    let (message, danger) = match outcome {
        Ok(done) => (format!("{action}: {done} of {} done.", ids.len()), false),
        Err(error) => (format!("{action} stopped: {error}"), true),
    };
    message_page(
        &state,
        &session,
        &resource.label,
        Some(&resource),
        message,
        danger,
    )
    .await
}

/// Returns how many rows the operation succeeded on, or the first error.
async fn run_bulk(
    state: &AppState,
    session: &session::Session,
    resource: &Resource,
    action: &str,
    ids: &[String],
    field: &impl Fn(&str) -> Option<String>,
) -> Result<usize, String> {
    let encoded: Vec<String> = ids.iter().map(|id| html::encode_path(id)).collect();
    let one_call = |path: String, body: Value| async move {
        hawkbit::send_json(state, session, Method::POST, &path, body)
            .await
            .map(|_| ids.len())
            .map_err(|error| error.message)
    };

    match (resource.id.as_str(), action) {
        // Upstream takes the whole list in one request.
        ("targets", "assign-ds") => {
            let ds = field("ds_id")
                .filter(|v| !v.is_empty())
                .ok_or("pick a distribution set")?;
            one_call(
                format!(
                    "distributionsets/{}/assignedTargets",
                    html::encode_path(&ds)
                ),
                json!(ids),
            )
            .await
        }
        ("targets", "tag") | ("targets", "untag") => {
            let tag = field("tag_id")
                .filter(|v| !v.is_empty())
                .ok_or("pick a tag")?;
            let path = format!("targettags/{}/assigned", html::encode_path(&tag));
            let method = if action == "tag" {
                Method::POST
            } else {
                Method::DELETE
            };
            hawkbit::send_json(state, session, method, &path, json!(ids))
                .await
                .map(|_| ids.len())
                .map_err(|error| error.message)
        }
        ("distribution-sets", "tag") | ("distribution-sets", "untag") => {
            let tag = field("tag_id")
                .filter(|v| !v.is_empty())
                .ok_or("pick a tag")?;
            let path = format!("distributionsettags/{}/assigned", html::encode_path(&tag));
            let method = if action == "tag" {
                Method::POST
            } else {
                Method::DELETE
            };
            let numeric: Vec<i64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
            hawkbit::send_json(state, session, method, &path, json!(numeric))
                .await
                .map(|_| ids.len())
                .map_err(|error| error.message)
        }
        // Both group endpoints take the whole selection in the body.
        ("targets", "set-group") => {
            let group = field("group")
                .filter(|group| !group.trim().is_empty())
                .ok_or("name the group")?;
            assign_group(state, session, Some(&group), ids)
                .await
                .map(|()| ids.len())
        }
        ("targets", "clear-group") => assign_group(state, session, None, ids)
            .await
            .map(|()| ids.len()),
        // Everything else is one request per row.
        _ => {
            let mut done = 0usize;
            let mut failures = Vec::new();
            for (id, encoded) in ids.iter().zip(&encoded) {
                let result = match action {
                    "delete" => {
                        hawkbit::send_empty(
                            state,
                            session,
                            Method::DELETE,
                            &format!("{}/{encoded}", resource.endpoint),
                        )
                        .await
                    }
                    "cancel-actions" => {
                        hawkbit::send_empty(
                            state,
                            session,
                            Method::DELETE,
                            &format!("targets/{encoded}/actions"),
                        )
                        .await
                    }
                    other => {
                        hawkbit::send_empty(
                            state,
                            session,
                            Method::POST,
                            &format!(
                                "{}/{encoded}/{}",
                                resource.endpoint,
                                html::encode_path(other)
                            ),
                        )
                        .await
                    }
                };
                match result {
                    Ok(_) => done += 1,
                    Err(error) => failures.push(format!("{id}: {}", error.message)),
                }
            }
            if failures.is_empty() {
                Ok(done)
            } else if done == 0 {
                Err(failures.join("; "))
            } else {
                // Partial success is still success for the rows that worked.
                tracing::warn!(failures = failures.len(), "bulk action partially failed");
                Ok(done)
            }
        }
    }
}

/// Rollout state machine: which commands the current status allows.
fn rollout_state_actions(resource: &Resource, id: &str, value: &Value) -> Vec<view::StateAction> {
    if resource.id != "rollouts" {
        return Vec::new();
    }
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    let available: &[&str] = match status {
        "ready" => &["start"],
        "running" => &["pause", "stop", "triggerNextGroup"],
        "paused" => &["resume", "stop"],
        "waiting_for_approval" => &["approve", "deny"],
        "finished" | "stopped" => &["retry"],
        _ => &[],
    };
    available
        .iter()
        .map(|action| view::StateAction {
            action: format!(
                "/{}/{}/actions/{action}",
                resource.id,
                html::encode_path(id)
            ),
            label: (*action).to_string(),
            confirm: format!("{action} rollout {id}?"),
            help: format!("deployment/rollout-lifecycle#{}", help::slug(action)),
        })
        .collect()
}

async fn detail_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id)): AxumPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let value = match hawkbit::get_json(
        &state,
        &session,
        &format!("{}/{}", resource.endpoint, html::encode_path(&id)),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            return message_page(
                &state,
                &session,
                &resource.label,
                Some(&resource),
                error.message,
                true,
            )
            .await
        }
    };
    // Sub-lists on a card page themselves; `?meta=` is the metadata pager.
    let meta_page = query
        .get("meta")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let extra = detail_extra(&state, &session, &resource, &id, &value, meta_page).await;
    view::respond(&view::DetailPage {
        layout: layout(
            &state,
            Some(&session),
            &resource.label,
            Some(&resource),
            None,
            None,
            "",
        )
        .await,
        label: resource.label.clone(),
        edit_href: resource
            .form
            .map(|_| format!("/{}/{}/edit", resource.id, html::encode_path(&id))),
        delete_action: format!("/{}/{}/delete", resource.id, html::encode_path(&id)),
        state_actions: rollout_state_actions(&resource, &id, &value),
        autorefresh: resource.id == "rollouts",
        fields: view::fields_of(&value, &resource.id),
        extra,
        id,
    })
}

async fn new_resource_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(resource_id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let mut values = HashMap::new();
    if let Some(ds) = query.get("ds") {
        // Preselect the DS when arriving from a DS page (?ds={id}).
        values.insert("distributionSetId".to_string(), ds.clone());
    }
    render_form(
        &state,
        &session,
        resource,
        FormRequest {
            values,
            next: query.get("next").cloned(),
            ..Default::default()
        },
    )
    .await
}

async fn create_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(resource_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let request_summary = format!("create {}", resource.label);
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: &resource.id,
            resource_id: "new",
            action: "create",
            request_summary,
        },
    )
    .await
    .ok();
    let body = match resources::build_payload(&resource, &payload, resources::PayloadMode::Create) {
        Ok(body) => body,
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.summary()).await;
            return form_error(
                &state,
                &session,
                resource,
                None,
                payload,
                error.field_errors,
                None,
            )
            .await;
        }
    };
    match hawkbit::send_json(&state, &session, Method::POST, &resource.endpoint, body).await {
        Ok(value) => {
            let id = html::created_row_id(&resource, &value);
            finish_log(
                &state,
                log.as_ref(),
                "success",
                &format!("created {id}"),
                "",
            )
            .await;
            // On-the-fly creation flows: return where the user came from.
            if let Some(next) = payload.get("next").filter(|n| n.starts_with('/')) {
                return Redirect::to(&next.replace("{id}", &html::encode_path(&id)))
                    .into_response();
            }
            Redirect::to(&format!("/{}/{}", resource.id, html::encode_path(&id))).into_response()
        }
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.message).await;
            form_error(
                &state,
                &session,
                resource,
                None,
                payload,
                HashMap::new(),
                Some(error.message),
            )
            .await
        }
    }
}

async fn edit_resource_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id)): AxumPath<(String, String)>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let mut values = HashMap::new();
    if let Ok(value) = hawkbit::get_json(
        &state,
        &session,
        &format!("{}/{}", resource.endpoint, html::encode_path(&id)),
    )
    .await
    {
        if let Some(form) = resource.form {
            for field in form.fields {
                if let Some(value) = html::read_path(&value, field.name) {
                    values.insert(field.name.to_string(), html::render_plain_value(value));
                }
            }
        }
    }
    render_form(
        &state,
        &session,
        resource,
        FormRequest {
            id: Some(&id),
            values,
            ..Default::default()
        },
    )
    .await
}

async fn update_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id)): AxumPath<(String, String)>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: &resource.id,
            resource_id: &id,
            action: "edit",
            request_summary: format!("edit {}", resource.label),
        },
    )
    .await
    .ok();
    let body = match resources::build_payload(&resource, &payload, resources::PayloadMode::Update) {
        Ok(body) => body,
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.summary()).await;
            return form_error(
                &state,
                &session,
                resource,
                Some(&id),
                payload,
                error.field_errors,
                None,
            )
            .await;
        }
    };
    let path = format!("{}/{}", resource.endpoint, html::encode_path(&id));
    match hawkbit::send_json(&state, &session, Method::PUT, &path, body).await {
        Ok(_) => {
            finish_log(&state, log.as_ref(), "success", "updated", "").await;
            Redirect::to(&format!("/{}/{}", resource.id, html::encode_path(&id))).into_response()
        }
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.message).await;
            form_error(
                &state,
                &session,
                resource,
                Some(&id),
                payload,
                HashMap::new(),
                Some(error.message),
            )
            .await
        }
    }
}

async fn delete_resource(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id)): AxumPath<(String, String)>,
) -> Response {
    resource_mutation(
        &state,
        &headers,
        &resource_id,
        &id,
        "delete",
        Method::DELETE,
        None,
    )
    .await
}

async fn resource_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id, action)): AxumPath<(String, String, String)>,
) -> Response {
    resource_mutation(
        &state,
        &headers,
        &resource_id,
        &id,
        &action,
        Method::POST,
        Some(&action),
    )
    .await
}

async fn resource_mutation(
    state: &AppState,
    headers: &HeaderMap,
    resource_id: &str,
    id: &str,
    action: &str,
    method: Method,
    action_path: Option<&str>,
) -> Response {
    let Some(resource) = resources::by_id(resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(state, headers).await else {
        return Redirect::to("/login").into_response();
    };
    let log = logs::record_started(
        state,
        &session,
        ActionLogDraft {
            resource: &resource.id,
            resource_id: id,
            action,
            request_summary: format!("{action} {}", resource.label),
        },
    )
    .await
    .ok();
    let path = action_path.map_or_else(
        || format!("{}/{}", resource.endpoint, html::encode_path(id)),
        |action| {
            format!(
                "{}/{}/{}",
                resource.endpoint,
                html::encode_path(id),
                html::encode_path(action)
            )
        },
    );
    match hawkbit::send_empty(state, &session, method, &path).await {
        Ok(_) => {
            finish_log(state, log.as_ref(), "success", "completed", "").await;
            let target = if action == "delete" {
                format!("/{}", resource.id)
            } else {
                format!("/{}/{}", resource.id, html::encode_path(id))
            };
            Redirect::to(&target).into_response()
        }
        Err(error) => {
            finish_log(state, log.as_ref(), "error", "", &error.message).await;
            message_page(
                state,
                &session,
                &resource.label,
                Some(&resource),
                error.message,
                true,
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Forms
// ---------------------------------------------------------------------------

/// Everything a create/edit form needs beyond the resource itself.
#[derive(Default)]
struct FormRequest<'a> {
    /// `None` on create.
    id: Option<&'a str>,
    values: HashMap<String, String>,
    field_errors: HashMap<String, String>,
    error: Option<String>,
    /// Where to go after a successful create (on-the-fly creation flows).
    next: Option<String>,
}

async fn render_form(
    state: &AppState,
    session: &session::Session,
    resource: Resource,
    request: FormRequest<'_>,
) -> Response {
    let FormRequest {
        id,
        values,
        field_errors,
        error,
        next,
    } = request;
    let layout = layout(
        state,
        Some(session),
        &resource.label,
        Some(&resource),
        None,
        None,
        "",
    )
    .await;
    let blocked = if id.is_none() && !resource.create_verified {
        Some("New is hidden until this hawkBit payload is verified against the Management API.")
    } else if resource.form.is_none() {
        Some("Create/edit is not supported for this resource yet.")
    } else {
        None
    };
    let spec = resource.form;
    let fields = spec
        .map(|spec| {
            spec.fields
                .iter()
                .map(|field| {
                    let value = values.get(field.name).cloned().unwrap_or_default();
                    let (input, options, picker) = match field.input {
                        resources::FieldInput::Text => (view::InputKind::Text, Vec::new(), None),
                        resources::FieldInput::TextArea => {
                            (view::InputKind::TextArea, Vec::new(), None)
                        }
                        resources::FieldInput::Checkbox => {
                            (view::InputKind::Checkbox, Vec::new(), None)
                        }
                        resources::FieldInput::Select(resources::SelectSource::Static(items)) => (
                            view::InputKind::Select,
                            std::iter::once(view::SelectOption {
                                value: String::new(),
                                label: String::new(),
                                selected: value.is_empty(),
                            })
                            .filter(|_| !field.required)
                            .chain(items.iter().map(|(option, label)| view::SelectOption {
                                value: (*option).to_string(),
                                label: (*label).to_string(),
                                selected: value == *option,
                            }))
                            .collect(),
                            None,
                        ),
                        resources::FieldInput::Select(
                            resources::SelectSource::DistributionSets,
                        ) => (
                            view::InputKind::Picker,
                            Vec::new(),
                            Some(
                                PickerSpec::new(
                                    field.name,
                                    "distribution-sets",
                                    "Find a distribution set…",
                                )
                                .with_current(value.clone(), value.clone()),
                            ),
                        ),
                    };
                    let help_key = help::form_field_key(spec.kind.help_topic(), field.name);
                    view::FormFieldView {
                        hint: help::summary_text(&help_key),
                        help: help_key,
                        label: field.label,
                        name: field.name,
                        required: field.required,
                        checked: value == "on" || value == "true",
                        value,
                        input,
                        options,
                        picker,
                        error: field_errors.get(field.name).cloned(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    view::respond(&view::FormPage {
        layout,
        title: spec
            .map(|spec| spec.title.to_string())
            .unwrap_or_else(|| resource.label.to_string()),
        action: match id {
            Some(id) => format!("/{}/{}/edit", resource.id, html::encode_path(id)),
            None => format!("/{}/new", resource.id),
        },
        next,
        error,
        blocked,
        fields,
    })
}

async fn form_error(
    state: &AppState,
    session: &session::Session,
    resource: Resource,
    id: Option<&str>,
    payload: HashMap<String, String>,
    field_errors: HashMap<String, String>,
    error: Option<String>,
) -> Response {
    render_form(
        state,
        session,
        resource,
        FormRequest {
            id,
            next: payload.get("next").cloned(),
            values: payload,
            field_errors,
            error,
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Saved views
// ---------------------------------------------------------------------------

async fn save_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let draft = view_draft(payload);
    let resource = draft.resource.clone();
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: &resource,
            resource_id: "view",
            action: "save-view",
            request_summary: "save user view".to_string(),
        },
    )
    .await
    .ok();
    match views::create(&state, &session, draft).await {
        Ok(_) => {
            finish_log(&state, log.as_ref(), "success", "saved view", "").await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.to_string()).await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
    }
}

async fn edit_view_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let Some(saved) = views::find_for_user(&state, &session, &id).await else {
        return not_found().await;
    };
    let resource = resources::by_id(&saved.resource);
    let encoded = html::encode_path(&saved.id);
    view::respond(&view::ViewFormPage {
        layout: layout(
            &state,
            Some(&session),
            "Edit View",
            resource.as_ref(),
            Some(&saved.id),
            None,
            "console/views#editing",
        )
        .await,
        title: "Edit View",
        action: format!("/views/{encoded}/edit"),
        resource: saved.resource.clone(),
        name: saved.name.clone(),
        q: saved.q.clone(),
        sort: saved.sort.clone(),
        columns: saved.columns.clone(),
        limit: saved.page_size,
        error: None,
        manage: Some(view::ViewManage {
            delete_action: format!("/views/{encoded}/delete"),
            duplicate_action: format!("/views/{encoded}/duplicate"),
            move_action: format!("/views/{encoded}/move"),
        }),
    })
}

async fn update_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let draft = view_draft(payload);
    let resource = draft.resource.clone();
    let log = log_view_action(&state, &session, &resource, "edit-view").await;
    match views::update(&state, &session, &id, draft).await {
        Ok(Some(saved)) => {
            finish_log(&state, log.as_ref(), "success", "updated view", "").await;
            Redirect::to(&format!(
                "/{}?view={}",
                saved.resource,
                html::encode_component(&saved.id)
            ))
            .into_response()
        }
        Ok(None) => not_found().await,
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.to_string()).await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
    }
}

async fn delete_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let resource = views::find_for_user(&state, &session, &id)
        .await
        .map(|view| view.resource)
        .unwrap_or_else(|| "targets".to_string());
    let log = log_view_action(&state, &session, &resource, "delete-view").await;
    match views::delete(&state, &session, &id).await {
        Ok(_) => {
            finish_log(&state, log.as_ref(), "success", "deleted view", "").await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.to_string()).await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
    }
}

async fn duplicate_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let resource = views::find_for_user(&state, &session, &id)
        .await
        .map(|view| view.resource)
        .unwrap_or_else(|| "targets".to_string());
    let log = log_view_action(&state, &session, &resource, "duplicate-view").await;
    match views::duplicate(&state, &session, &id).await {
        Ok(Some(saved)) => {
            finish_log(&state, log.as_ref(), "success", "duplicated view", "").await;
            Redirect::to(&format!(
                "/{}?view={}",
                saved.resource,
                html::encode_component(&saved.id)
            ))
            .into_response()
        }
        Ok(None) => not_found().await,
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.to_string()).await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
    }
}

async fn move_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Form(payload): Form<MoveViewForm>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let resource = views::find_for_user(&state, &session, &id)
        .await
        .map(|view| view.resource)
        .unwrap_or_else(|| "targets".to_string());
    let log = log_view_action(&state, &session, &resource, "move-view").await;
    match views::move_view(&state, &session, &id, &payload.direction).await {
        Ok(Some(saved)) => {
            finish_log(&state, log.as_ref(), "success", "moved view", "").await;
            Redirect::to(&format!(
                "/{}?view={}",
                saved.resource,
                html::encode_component(&saved.id)
            ))
            .into_response()
        }
        Ok(None) => not_found().await,
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.to_string()).await;
            Redirect::to(&format!("/{resource}")).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

async fn search_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let q = query.q.unwrap_or_default();
    crate::searches::record(&state, &session, &q).await;
    let sections = search_sections(&state, &session, &q, 50, 25).await;
    view::respond(&view::SearchPage {
        layout: layout(
            &state,
            Some(&session),
            "Search",
            None,
            None,
            Some(&q),
            "console/search",
        )
        .await,
        empty: sections.iter().all(|section| section.rows.is_empty()),
        q,
        sections,
    })
}

async fn live_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return (StatusCode::UNAUTHORIZED, Html("Login required".to_string())).into_response();
    };
    let q = query.q.unwrap_or_default();
    if q.trim().is_empty() {
        // Nothing typed yet: offer what this user searched for before.
        let recent = crate::searches::recent(&state, &session, 8).await;
        return view::respond(&view::RecentSearchesFragment {
            recent: recent
                .into_iter()
                .map(|entry| view::RecentSearchRow {
                    href: format!("/search?q={}", html::encode_component(&entry.q)),
                    delete_action: format!(
                        "/search/history/{}/delete",
                        html::encode_path(&entry.id)
                    ),
                    q: entry.q,
                })
                .collect(),
        });
    }
    if q.trim().len() < 2 {
        return view::respond(&view::LiveSearchFragment {
            sections: Vec::new(),
        });
    }
    view::respond(&view::LiveSearchFragment {
        sections: search_sections(&state, &session, &q, 8, 5).await,
    })
}

/// Typeahead behind `[data-picker]`: the same server-side RSQL the global
/// search uses, capped so no popup ever grows without bound.
async fn pick_options(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(resource_id): AxumPath<String>,
    Query(query): Query<SearchQuery>,
) -> Response {
    const PICK_LIMIT: usize = 20;
    let Some(resource) = resources::by_id(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return (StatusCode::UNAUTHORIZED, Html("Login required".to_string())).into_response();
    };
    let needle = hawkbit::rsql_literal(&query.q.unwrap_or_default());
    let filter = search_query(&resource, &needle);
    let page = hawkbit::fetch_page(
        &state,
        &session,
        &resource.endpoint,
        filter.as_deref(),
        // Pickers are alphabetical: you are looking for a name you know.
        Some("name:ASC"),
        1,
        PICK_LIMIT + 1,
    )
    .await
    .map(|value| hawkbit::read_items(&value))
    .unwrap_or_default();
    let truncated = page.len() > PICK_LIMIT;
    view::respond(&view::PickFragment {
        rows: page
            .iter()
            .take(PICK_LIMIT)
            .map(|item| view::PickRow {
                value: html::row_id(&resource, item),
                label: html::row_label(&resource, item),
            })
            .collect(),
        truncated,
    })
}

async fn delete_search_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(_session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    crate::searches::delete(&state, &id).await;
    Redirect::to("/search").into_response()
}

async fn clear_search_history(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    crate::searches::clear(&state, &session).await;
    Redirect::to("/search").into_response()
}

/// `field=="*needle*",field2=="*needle*"` — OR across a resource's text fields.
/// Quoted: a needle with spaces or the words `and`/`or` stays text.
fn search_query(resource: &Resource, needle: &str) -> Option<String> {
    if needle.is_empty() || resource.search_fields.is_empty() {
        return None;
    }
    Some(
        resource
            .search_fields
            .iter()
            .map(|field| format!("{field}==\"*{needle}*\""))
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Global search. The filtering happens **on the server** (RSQL wildcards over
/// each resource's text fields) — scanning the first page in memory used to
/// miss everything outside the 50 oldest rows.
async fn search_sections(
    state: &AppState,
    session: &session::Session,
    q: &str,
    resource_limit: usize,
    log_limit: usize,
) -> Vec<view::SearchSection> {
    if q.trim().is_empty() {
        return Vec::new();
    }
    let needle = hawkbit::rsql_literal(q);

    // One round-trip per resource, all in flight at once.
    let searchable = resources::all();
    let lookups = futures::future::join_all(searchable.iter().map(|resource| {
        let query = search_query(resource, &needle);
        async move {
            let rows = match query {
                Some(query) => hawkbit::fetch_page(
                    state,
                    session,
                    &resource.endpoint,
                    Some(&query),
                    resource.default_sort.as_deref(),
                    1,
                    resource_limit,
                )
                .await
                .map(|value| {
                    hawkbit::read_items(&value)
                        .iter()
                        .map(|item| view::SearchHit {
                            href: html::row_href(resource, item),
                            label: html::row_label(resource, item),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
                None => Vec::new(),
            };
            view::SearchSection {
                title: resource.label.to_string(),
                rows,
            }
        }
    }));

    let (mut sections, logs) = tokio::join!(lookups, logs::search(state, session, q, log_limit));
    sections.push(view::SearchSection {
        title: "Action Log".to_string(),
        rows: logs
            .into_iter()
            .map(|log| view::SearchHit {
                href: "/action-log".to_string(),
                label: format!(
                    "{} {} {}",
                    view::text_of(&log, "resource"),
                    view::text_of(&log, "action"),
                    view::text_of(&log, "status")
                ),
            })
            .collect(),
    });
    sections
}

// ---------------------------------------------------------------------------
// Detail panels
// ---------------------------------------------------------------------------

fn artifacts_panel(sm_id: &str, artifacts: &[Value], next: &str) -> view::ArtifactsPanel {
    let sm_path = html::encode_path(sm_id);
    view::ArtifactsPanel {
        upload_action: format!("/software-modules/{sm_path}/artifacts"),
        next: next.to_string(),
        rows: artifacts
            .iter()
            .map(|artifact| {
                let sha1 = artifact
                    .pointer("/hashes/sha1")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let artifact_id = view::id_of(artifact, "id");
                view::ArtifactRow {
                    download_href: format!(
                        "/software-modules/{sm_path}/artifacts/{artifact_id}/download"
                    ),
                    name: view::str_or(artifact, "providedFilename", "-"),
                    size: view::id_of(artifact, "size"),
                    sha1_short: sha1.chars().take(12).collect(),
                    sha1,
                    delete_action: format!(
                        "/software-modules/{sm_path}/artifacts/{artifact_id}/delete"
                    ),
                }
            })
            .collect(),
    }
}

fn tags_panel(
    base: &str,
    create_href: &'static str,
    picker_resource: &'static str,
    tags: &[Value],
) -> view::TagsPanel {
    view::TagsPanel {
        chips: tags
            .iter()
            .map(|tag| view::ChipRow {
                label: view::str_or(tag, "name", "-"),
                colour: view::text_of(tag, "colour"),
                remove_action: format!("{base}/tags/{}/remove", view::id_of(tag, "id")),
            })
            .collect(),
        add_action: format!("{base}/tags"),
        picker: PickerSpec::new("tag_id", picker_resource, "Find a tag…"),
        create_href,
    }
}

/// Metadata sub-resource: free-form key/value pairs on an object. Paged like
/// any other list — an object can carry hundreds of them.
async fn metadata_panel(
    state: &AppState,
    session: &session::Session,
    resource: &Resource,
    id: &str,
    page: usize,
) -> view::MetadataPanel {
    const PER_PAGE: usize = 20;
    let base = format!("{}/{}", resource.endpoint, html::encode_path(id));
    let listing = hawkbit::get_json(
        state,
        session,
        &format!(
            "{base}/metadata?limit={PER_PAGE}&offset={}",
            (page - 1) * PER_PAGE
        ),
    )
    .await
    .unwrap_or_else(|_| json!({}));
    let rows = hawkbit::read_items(&listing);
    let total = hawkbit::read_total(&listing).unwrap_or(rows.len());
    let self_url = format!("/{}/{}", resource.id, html::encode_path(id));
    view::MetadataPanel {
        save_action: format!("{self_url}/metadata"),
        delete_action: format!("{self_url}/metadata/delete"),
        shows_target_visible: resource.id == "software-modules",
        pager: view::Pager::new(page, PER_PAGE, total, rows.len(), &|target| {
            format!("{self_url}?meta={target}")
        }),
        rows: rows
            .iter()
            .map(|row| view::MetadataRow {
                key: view::text_of(row, "key"),
                value: view::text_of(row, "value"),
                target_visible: row
                    .get("targetVisible")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
            .collect(),
    }
}

/// How many devices carry this set, assigned and installed. Counted with the
/// very query each card links to (UI/UX rule 6) rather than with
/// `/distributionsets/{id}/statistics`, which answers a different question
/// and cannot be filtered or paged.
async fn ds_usage(state: &AppState, session: &session::Session, id: &str) -> Vec<view::StatCard> {
    let queries = [
        ("assigned", format!("assigneddistributionset=={id}")),
        ("installed", format!("installeddistributionset=={id}")),
    ];
    let counts = futures::future::join_all(
        queries
            .iter()
            .map(|(_, q)| hawkbit::count_resource(state, session, "targets", Some(q))),
    )
    .await;
    queries
        .iter()
        .zip(counts)
        .map(|((label, q), value)| view::StatCard {
            href: format!("/targets?q={}", html::encode_component(q)),
            label: (*label).to_string(),
            count: view::count_text(value.get("count").and_then(Value::as_u64)),
        })
        .collect()
}

async fn detail_extra(
    state: &AppState,
    session: &session::Session,
    resource: &Resource,
    id: &str,
    value: &Value,
    meta_page: usize,
) -> DetailExtra {
    let self_url = format!("/{}/{}", resource.id, html::encode_path(id));
    match resource.id.as_str() {
        "rollouts" => {
            let groups = hawkbit::get_json(
                state,
                session,
                &format!("rollouts/{}/deploygroups?limit=100", html::encode_path(id)),
            )
            .await
            .map(|value| hawkbit::read_items(&value))
            .unwrap_or_default();
            let total = value
                .get("totalTargets")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(1);
            let per_status = value.get("totalTargetsPerStatus");
            let rollout_path = html::encode_path(&view::id_of(value, "id"));
            DetailExtra::Rollout(view::RolloutPanel {
                bars: [
                    ("finished", "ok"),
                    ("running", "info"),
                    ("scheduled", "muted"),
                    ("notstarted", "muted"),
                    ("error", "danger"),
                    ("cancelled", "warn"),
                ]
                .into_iter()
                .filter_map(|(key, class)| {
                    let count = per_status
                        .and_then(|status| status.get(key))
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    (count > 0).then_some(view::ProgressBar {
                        label: key,
                        class,
                        percent: (count * 100 / total).clamp(1, 100),
                        count,
                    })
                })
                .collect(),
                groups: groups
                    .iter()
                    .map(|group| view::GroupRow {
                        href: format!(
                            "/rollouts/{rollout_path}/groups/{}",
                            view::id_of(group, "id")
                        ),
                        name: view::str_or(group, "name", "-"),
                        status: view::str_or(group, "status", "-"),
                        total: group
                            .get("totalTargets")
                            .and_then(Value::as_i64)
                            .unwrap_or(0),
                        success: group
                            .pointer("/successCondition/expression")
                            .and_then(Value::as_str)
                            .unwrap_or("-")
                            .to_string(),
                        error: group
                            .pointer("/errorCondition/expression")
                            .and_then(Value::as_str)
                            .unwrap_or("-")
                            .to_string(),
                    })
                    .collect(),
            })
        }
        "software-modules" => {
            let artifacts = hawkbit::get_json(
                state,
                session,
                &format!("softwaremodules/{}/artifacts", html::encode_path(id)),
            )
            .await
            .map(|value| value.as_array().cloned().unwrap_or_default())
            .unwrap_or_default();
            DetailExtra::SoftwareModule(view::SmPanel {
                artifacts: artifacts_panel(id, &artifacts, &self_url),
                metadata: metadata_panel(state, session, resource, id, meta_page).await,
            })
        }
        "distribution-sets" => {
            let ds_path = html::encode_path(id);
            let modules = hawkbit::get_json(
                state,
                session,
                &format!("distributionsets/{ds_path}/assignedSM?limit=100"),
            )
            .await
            .map(|value| hawkbit::read_items(&value))
            .unwrap_or_default();
            let mut nodes = Vec::new();
            for module in &modules {
                let sm_id = view::id_of(module, "id");
                let artifacts = hawkbit::get_json(
                    state,
                    session,
                    &format!("softwaremodules/{sm_id}/artifacts"),
                )
                .await
                .map(|value| value.as_array().cloned().unwrap_or_default())
                .unwrap_or_default();
                nodes.push(view::ModuleNode {
                    name: view::str_or(module, "name", "-"),
                    version: view::text_of(module, "version"),
                    kind: view::text_of(module, "type"),
                    detach_action: format!("/distribution-sets/{ds_path}/modules/{sm_id}/detach"),
                    artifacts: artifacts_panel(&sm_id, &artifacts, &self_url),
                });
            }
            let tags =
                hawkbit::get_json(state, session, &format!("distributionsets/{ds_path}/tags"))
                    .await
                    .map(|value| hawkbit::read_items(&value))
                    .unwrap_or_default();
            DetailExtra::DistributionSet(view::DsPanel {
                usage: ds_usage(state, session, id).await,
                metadata: metadata_panel(state, session, resource, id, meta_page).await,
                modules: nodes,
                attach_action: format!("/distribution-sets/{ds_path}/modules"),
                attach_picker: PickerSpec::new(
                    "sm_id",
                    "software-modules",
                    "Find a software module…",
                ),
                create_module_href: format!(
                    "/software-modules/new?next={}",
                    html::encode_component(&self_url)
                ),
                rollout_href: format!("/rollouts/new?ds={ds_path}"),
                next: self_url.clone(),
                tags: tags_panel(
                    &self_url,
                    "/distribution-set-tags/new",
                    "distribution-set-tags",
                    &tags,
                ),
            })
        }
        "incidents" => {
            // The generic page prints the journal fields; this panel adds
            // what only the number knows, plus the console tasks that
            // mention it.
            let related = logs::search(state, session, id, 20).await;
            DetailExtra::Incident(view::IncidentPanel {
                number: view::text_of(value, "incident"),
                recorded: value
                    .get("occurredAt")
                    .and_then(Value::as_i64)
                    .map(Cell::time_full)
                    .unwrap_or_else(|| Cell::Muted("unknown".to_string())),
                node: view::id_of(value, "node"),
                sequence: view::id_of(value, "sequence"),
                logs: related
                    .iter()
                    .map(|log| view::LogRow {
                        time: log
                            .get("createdAt")
                            .and_then(Value::as_i64)
                            .map(Cell::time)
                            .unwrap_or_else(|| Cell::Muted("—".to_string())),
                        task: format!(
                            "{} {}",
                            view::text_of(log, "action"),
                            view::text_of(log, "resource")
                        ),
                        status: view::text_of(log, "status"),
                        error: view::text_of(log, "error"),
                    })
                    .collect(),
            })
        }
        "distribution-set-types" => {
            let type_path = html::encode_path(id);
            let mandatory_path = format!("distributionsettypes/{type_path}/mandatorymoduletypes");
            let optional_path = format!("distributionsettypes/{type_path}/optionalmoduletypes");
            let (mandatory, optional) = tokio::join!(
                hawkbit::get_json(state, session, &mandatory_path),
                hawkbit::get_json(state, session, &optional_path),
            );
            let chips = |rows: Vec<Value>, kind: &str| -> Vec<view::ChipRow> {
                rows.iter()
                    .map(|item| view::ChipRow {
                        label: view::str_or(item, "name", "-"),
                        colour: String::new(),
                        remove_action: format!(
                            "/distribution-set-types/{type_path}/modules/{kind}/{}/remove",
                            view::id_of(item, "id")
                        ),
                    })
                    .collect()
            };
            DetailExtra::DsType(view::DsTypePanel {
                mandatory: chips(
                    mandatory
                        .map(|v| hawkbit::read_items(&v))
                        .unwrap_or_default(),
                    "mandatory",
                ),
                optional: chips(
                    optional
                        .map(|v| hawkbit::read_items(&v))
                        .unwrap_or_default(),
                    "optional",
                ),
                add_action: format!("/distribution-set-types/{type_path}/modules"),
                picker: PickerSpec::new(
                    "sm_type_id",
                    "software-module-types",
                    "Find a module type…",
                ),
            })
        }
        "targets" => {
            let cid = html::encode_path(id);
            let attributes_path = format!("targets/{cid}/attributes");
            let actions_path = format!("targets/{cid}/actions?limit=10");
            let tags_path = format!("targets/{cid}/tags");
            let (attributes, actions, tags) = tokio::join!(
                hawkbit::get_json(state, session, &attributes_path),
                hawkbit::get_json(state, session, &actions_path),
                hawkbit::get_json(state, session, &tags_path),
            );
            let attributes = attributes.unwrap_or_else(|_| json!({}));
            let actions = actions
                .map(|value| hawkbit::read_items(&value))
                .unwrap_or_default();
            let tags = tags
                .map(|value| hawkbit::read_items(&value))
                .unwrap_or_default();
            let current_type = value
                .get("targetType")
                .filter(|value| !value.is_null())
                .map(html::render_plain_value)
                .unwrap_or_default();
            DetailExtra::Target(view::TargetPanel {
                metadata: metadata_panel(state, session, resource, id, meta_page).await,
                group: view::GroupPanel {
                    current_href: html::filtered_list_href(
                        "target-groups",
                        "name",
                        &view::text_of(value, "group"),
                    ),
                    current: view::text_of(value, "group"),
                    set_action: format!("/targets/{cid}/group"),
                    clear_action: format!("/targets/{cid}/group/clear"),
                    picker: PickerSpec::new("group", "target-groups", "Find or type a path…")
                        .free_text(),
                },
                attributes: attributes
                    .as_object()
                    .map(|object| {
                        object
                            .iter()
                            .map(|(key, value)| view::Field {
                                key: key.clone(),
                                cell: view::value_cell(Some(value)),
                                raw: html::raw_value(value),
                                help: "inventory/identity#attributes".to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                token: view::text_of(value, "securityToken"),
                assign_action: format!("/targets/{cid}/assign"),
                assign_picker: PickerSpec::new(
                    "ds_id",
                    "distribution-sets",
                    "Find a distribution set…",
                ),
                tags: tags_panel(&self_url, "/target-tags/new", "target-tags", &tags),
                type_action: format!("/targets/{cid}/type"),
                type_picker: PickerSpec::new(
                    "type_id",
                    "target-types",
                    "Find a target type (empty clears it)…",
                )
                .with_current(current_type, view::text_of(value, "targetTypeName")),
                auto_confirm_active: value
                    .get("autoConfirmActive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                auto_confirm_action: if value
                    .get("autoConfirmActive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    format!("/targets/{cid}/auto-confirm/deactivate")
                } else {
                    format!("/targets/{cid}/auto-confirm/activate")
                },
                actions: actions
                    .iter()
                    .map(|action| {
                        let action_id = view::id_of(action, "id");
                        let pending =
                            action.get("status").and_then(Value::as_str) == Some("pending");
                        view::ActionRow {
                            href: format!("/targets/{cid}/actions/{action_id}"),
                            detail: view::str_or(action, "detailStatus", "-"),
                            kind: view::str_or(action, "type", "-"),
                            status: view::str_or(action, "status", "-"),
                            cancel_action: pending
                                .then(|| format!("/targets/{cid}/actions/{action_id}/cancel")),
                            id: action_id,
                        }
                    })
                    .collect(),
            })
        }
        "target-types" => {
            let type_path = html::encode_path(id);
            let compatible = hawkbit::get_json(
                state,
                session,
                &format!("targettypes/{type_path}/compatibledistributionsettypes"),
            )
            .await
            .map(|value| hawkbit::read_items(&value))
            .unwrap_or_default();
            DetailExtra::TargetType(view::TargetTypePanel {
                compatible: compatible
                    .iter()
                    .map(|item| view::ChipRow {
                        label: view::str_or(item, "name", "-"),
                        colour: String::new(),
                        remove_action: format!(
                            "/target-types/{type_path}/ds-types/{}/remove",
                            view::id_of(item, "id")
                        ),
                    })
                    .collect(),
                add_action: format!("/target-types/{type_path}/ds-types"),
                picker: PickerSpec::new(
                    "ds_type_id",
                    "distribution-set-types",
                    "Find a distribution set type…",
                ),
            })
        }
        "target-filters" => {
            let filter_path = html::encode_path(id);
            let ds_id = value
                .get("autoAssignDistributionSet")
                .filter(|value| !value.is_null())
                .map(html::render_plain_value);
            DetailExtra::TargetFilter(view::AutoAssignPanel {
                set_action: format!("/target-filters/{filter_path}/auto-assign"),
                clear_action: format!("/target-filters/{filter_path}/auto-assign/clear"),
                picker: PickerSpec::new("ds_id", "distribution-sets", "Find a distribution set…"),
                current: ds_id.map(|ds_id| view::AutoAssignCurrent {
                    href: format!("/distribution-sets/{}", html::encode_path(&ds_id)),
                    label: view::str_or(value, "autoAssignDistributionSetName", &ds_id),
                    action_type: view::str_or(value, "autoAssignActionType", "forced"),
                    weight: value
                        .get("autoAssignWeight")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    confirmation_required: value
                        .get("confirmationRequired")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                }),
            })
        }
        _ => DetailExtra::None,
    }
}

// ---------------------------------------------------------------------------
// Rollout group and action pages
// ---------------------------------------------------------------------------

async fn rollout_group_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((rollout_id, group_id)): AxumPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let rollout_path = html::encode_path(&rollout_id);
    let group_path = html::encode_path(&group_id);
    let group = hawkbit::get_json(
        &state,
        &session,
        &format!("rollouts/{rollout_path}/deploygroups/{group_path}"),
    )
    .await
    .unwrap_or_else(|_| json!({}));

    // A group can hold thousands of devices: page it like any other list.
    let page = query
        .get("page")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let limit = 50usize;
    // Exactly one action per (group, target), so the group's actions *are*
    // its devices — and each row can go straight to its status history.
    let listing = hawkbit::get_json(
        &state,
        &session,
        &format!(
            "actions?q={}&sort=id:ASC&limit={limit}&offset={}",
            html::encode_component(&format!("rolloutgroup.id=={group_id}")),
            (page - 1) * limit,
        ),
    )
    .await
    .unwrap_or_else(|_| json!({}));
    let actions = hawkbit::read_items(&listing);
    let total = hawkbit::read_total(&listing).unwrap_or(actions.len());

    view::respond(&view::RolloutGroupPage {
        layout: layout(
            &state,
            Some(&session),
            "Rollout group",
            resources::by_id("rollouts").as_ref(),
            None,
            None,
            "deployment/rollout-groups",
        )
        .await,
        name: view::str_or(&group, "name", "-"),
        rollout_href: format!("/rollouts/{rollout_path}"),
        rollout_id,
        status: view::str_or(&group, "status", "-"),
        total: group
            .get("totalTargets")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        pager: view::Pager::new(page, limit, total, actions.len(), &|target| {
            format!("/rollouts/{rollout_path}/groups/{group_path}?page={target}")
        }),
        rows: actions
            .iter()
            .map(|action| {
                let controller_id = view::text_of(action, "controllerId");
                view::GroupActionRow {
                    href: format!(
                        "/targets/{}/actions/{}",
                        html::encode_path(&controller_id),
                        view::id_of(action, "id")
                    ),
                    controller_id,
                    detail: view::str_or(action, "detailStatus", "-"),
                    kind: view::str_or(action, "type", "-"),
                    status: view::str_or(action, "status", "-"),
                    created: action
                        .get("createdAt")
                        .and_then(Value::as_i64)
                        .map(Cell::time)
                        .unwrap_or_else(|| Cell::Muted("—".to_string())),
                }
            })
            .collect(),
    })
}

async fn action_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((controller_id, action_id)): AxumPath<(String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let cid = html::encode_path(&controller_id);
    let aid = html::encode_path(&action_id);
    let action_path = format!("targets/{cid}/actions/{aid}");
    let status_path = format!("targets/{cid}/actions/{aid}/status?limit=100");
    let (action, statuses) = tokio::join!(
        hawkbit::get_json(&state, &session, &action_path),
        hawkbit::get_json(&state, &session, &status_path),
    );
    let action = action.unwrap_or_else(|_| json!({}));
    let statuses = statuses
        .map(|value| hawkbit::read_items(&value))
        .unwrap_or_default();

    view::respond(&view::ActionPage {
        layout: layout(
            &state,
            Some(&session),
            "Action",
            resources::by_id("targets").as_ref(),
            None,
            None,
            "deployment/actions",
        )
        .await,
        action_id: view::id_of(&action, "id"),
        target_href: format!("/targets/{cid}"),
        cancellable: action.get("status").and_then(Value::as_str) == Some("pending"),
        cancel_action: format!("/targets/{cid}/actions/{aid}/cancel"),
        detail: view::str_or(&action, "detailStatus", "-"),
        status: view::str_or(&action, "status", "-"),
        kind: view::str_or(&action, "type", "-"),
        // A named reference is a link, not text (rule 17). The id travels
        // next to the name in the link object precisely so the console does
        // not have to parse it back out of the href.
        distribution_set: match (
            action
                .pointer("/_links/distributionset/name")
                .and_then(Value::as_str),
            action
                .pointer("/_links/distributionset/id")
                .and_then(Value::as_i64),
        ) {
            (Some(name), Some(id)) => Cell::Link {
                href: format!("/distribution-sets/{id}"),
                label: name.to_string(),
            },
            (Some(name), None) => Cell::Text(name.to_string()),
            _ => Cell::Muted("—".to_string()),
        },
        statuses: statuses
            .iter()
            .map(|status| view::StatusRow {
                time: status
                    .get("reportedAt")
                    .and_then(Value::as_i64)
                    .map(Cell::time)
                    .unwrap_or_else(|| Cell::Muted("—".to_string())),
                reports: status
                    .get("repeats")
                    .and_then(Value::as_i64)
                    .unwrap_or_default()
                    + 1,
                last_time: status
                    .get("lastReportedAt")
                    .and_then(Value::as_i64)
                    .map(Cell::time)
                    .unwrap_or_else(|| Cell::Muted("—".to_string())),
                kind: view::str_or(status, "type", "-"),
                code: status
                    .get("code")
                    .filter(|code| !code.is_null())
                    .map(html::render_plain_value)
                    .unwrap_or_default(),
                messages: status
                    .get("messages")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
        controller_id,
    })
}

// ---------------------------------------------------------------------------
// JSON endpoints and the upstream proxy
// ---------------------------------------------------------------------------

async fn static_asset(State(state): State<AppState>, AxumPath(path): AxumPath<String>) -> Response {
    html::static_asset(&state, &path).await
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "hawier-ui",
        build: BUILD,
        hawkbit_mgmt_url: state.config.hawkbit_mgmt_url,
    })
}

async fn api_session(State(state): State<AppState>, headers: HeaderMap) -> Json<SessionResponse> {
    let session = session::current(&state, &headers).await;
    Json(SessionResponse {
        authenticated: session.is_some(),
        username: session.as_ref().map(|s| s.username.clone()),
        tenant: session.as_ref().map(|s| s.tenant.clone()),
    })
}

async fn api_login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let (session, cookie) = session::authenticate(&state, request).await?;
    let mut response = Json(SessionResponse {
        authenticated: true,
        username: Some(session.username),
        tenant: Some(session.tenant),
    })
    .into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    Ok(response)
}

async fn api_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    session::clear(
        &state,
        &headers,
        Json(json!({ "authenticated": false })).into_response(),
    )
    .await
}

async fn api_hawkbit_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let session = session::require(&state, &headers).await?;
    Ok(Json(hawkbit::status(&state, &session).await?).into_response())
}

async fn api_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let session = session::require(&state, &headers).await?;
    let mut result = serde_json::Map::new();
    for resource in resources::all() {
        let value = hawkbit::count_resource(&state, &session, &resource.endpoint, None).await;
        result.insert(resource.id.to_string(), value);
    }
    Ok(Json(Value::Object(result)).into_response())
}

async fn proxy_hawkbit(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    AxumPath(path): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Result<Response, AppError> {
    let session = session::require(&state, &headers).await?;
    hawkbit::proxy(
        &state,
        &session,
        hawkbit::ProxyRequest {
            headers,
            method,
            uri,
            path,
            query,
            body,
        },
    )
    .await
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

fn view_draft(payload: HashMap<String, String>) -> SavedViewDraft {
    SavedViewDraft {
        resource: payload.get("resource").cloned().unwrap_or_default(),
        name: payload
            .get("name")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Custom view".to_string()),
        q: payload.get("q").cloned().unwrap_or_default(),
        columns: payload.get("columns").cloned().unwrap_or_default(),
        sort: payload.get("sort").cloned().unwrap_or_default(),
        page_size: payload
            .get("limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(25),
    }
}

async fn log_view_action(
    state: &AppState,
    session: &session::Session,
    resource: &str,
    action: &str,
) -> Option<logs::ActionLog> {
    logs::record_started(
        state,
        session,
        ActionLogDraft {
            resource,
            resource_id: "view",
            action,
            request_summary: action.to_string(),
        },
    )
    .await
    .ok()
}

async fn finish_log(
    state: &AppState,
    log: Option<&logs::ActionLog>,
    status: &str,
    response: &str,
    error: &str,
) {
    if let Some(log) = log {
        let _ = logs::finish(
            state,
            &log.id,
            status,
            response.to_string(),
            error.to_string(),
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Artifact upload / delete, DS module attach/detach, target assign
// ---------------------------------------------------------------------------

fn next_or(payload: &HashMap<String, String>, fallback: String) -> String {
    payload
        .get("next")
        .filter(|n| n.starts_with('/'))
        .cloned()
        .unwrap_or(fallback)
}

async fn upload_artifact_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(sm_id): AxumPath<String>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let mut next = format!("/software-modules/{}", html::encode_path(&sm_id));
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "software-modules",
            resource_id: &sm_id,
            action: "upload-artifact",
            request_summary: format!("upload artifact to SM {sm_id}"),
        },
    )
    .await
    .ok();

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("next") => {
                if let Ok(value) = field.text().await {
                    if value.starts_with('/') {
                        next = value;
                    }
                }
            }
            Some("file") => {
                let filename = field
                    .file_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "artifact.bin".to_string());
                match hawkbit::upload_artifact(&state, &session, &sm_id, &filename, field).await {
                    Ok(value) => {
                        let artifact_id = value
                            .get("id")
                            .map(html::render_plain_value)
                            .unwrap_or_default();
                        finish_log(
                            &state,
                            log.as_ref(),
                            "success",
                            &format!("artifact {artifact_id} ({filename})"),
                            "",
                        )
                        .await;
                        return Redirect::to(&next).into_response();
                    }
                    Err(error) => {
                        finish_log(&state, log.as_ref(), "error", "", &error.message).await;
                        return message_page(
                            &state,
                            &session,
                            "Upload failed",
                            resources::by_id("software-modules").as_ref(),
                            error.message,
                            true,
                        )
                        .await;
                    }
                }
            }
            _ => {}
        }
    }
    finish_log(&state, log.as_ref(), "error", "", "no file provided").await;
    Redirect::to(&next).into_response()
}

async fn delete_artifact_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((sm_id, artifact_id)): AxumPath<(String, String)>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = next_or(
        &payload,
        format!("/software-modules/{}", html::encode_path(&sm_id)),
    );
    let _ = hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "softwaremodules/{}/artifacts/{}",
            html::encode_path(&sm_id),
            html::encode_path(&artifact_id)
        ),
    )
    .await;
    Redirect::to(&next).into_response()
}

async fn attach_module(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(ds_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = next_or(
        &payload,
        format!("/distribution-sets/{}", html::encode_path(&ds_id)),
    );
    if let Some(sm_id) = payload.get("sm_id").and_then(|v| v.parse::<i64>().ok()) {
        let _ = hawkbit::send_json(
            &state,
            &session,
            Method::POST,
            &format!("distributionsets/{}/assignedSM", html::encode_path(&ds_id)),
            serde_json::json!([{ "id": sm_id }]),
        )
        .await;
    }
    Redirect::to(&next).into_response()
}

async fn detach_module(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((ds_id, sm_id)): AxumPath<(String, String)>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = next_or(
        &payload,
        format!("/distribution-sets/{}", html::encode_path(&ds_id)),
    );
    let _ = hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "distributionsets/{}/assignedSM/{}",
            html::encode_path(&ds_id),
            html::encode_path(&sm_id)
        ),
    )
    .await;
    Redirect::to(&next).into_response()
}

async fn update_tenant_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let Some(key) = payload.get("key").filter(|k| !k.trim().is_empty()) else {
        return Redirect::to("/settings").into_response();
    };
    let value = payload.get("value").cloned().unwrap_or_default();
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "settings",
            resource_id: key,
            action: "config-update",
            request_summary: format!("set {key} = {value}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_json(
        &state,
        &session,
        Method::PUT,
        &format!("system/configs/{}", html::encode_path(key)),
        json!({ "value": value }),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "updated", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to("/settings").into_response()
}

/// Set one extension parameter on the node this console talks to.
///
/// The value is never written into the action log: that log is a readable
/// mgmt resource, and `update_tenant_config` above does log its value —
/// repeating that here would put an S3 secret key in the database.
async fn update_ext_param(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let Some(name) = payload.get("name").filter(|n| !n.trim().is_empty()) else {
        return Redirect::to("/settings").into_response();
    };
    let value = payload.get("value").cloned().unwrap_or_default();
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "settings",
            resource_id: name,
            action: "extension-parameter-update",
            request_summary: format!("set {name}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_json(
        &state,
        &session,
        Method::PUT,
        &format!(
            "system/extensions/parameters/{}",
            html::encode_path(name.trim())
        ),
        json!({ "value": value }),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "updated", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to("/settings").into_response()
}

async fn delete_ext_param(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let Some(name) = payload.get("name").filter(|n| !n.trim().is_empty()) else {
        return Redirect::to("/settings").into_response();
    };
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "settings",
            resource_id: name,
            action: "extension-parameter-delete",
            request_summary: format!("remove {name}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_json(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "system/extensions/parameters/{}",
            html::encode_path(name.trim())
        ),
        json!({}),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "removed", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to("/settings").into_response()
}

async fn delete_tenant_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(key) = payload.get("key").filter(|k| !k.trim().is_empty()) {
        let _ = hawkbit::send_empty(
            &state,
            &session,
            Method::DELETE,
            &format!("system/configs/{}", html::encode_path(key)),
        )
        .await;
    }
    Redirect::to("/settings").into_response()
}

async fn add_target_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(controller_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/targets/{}", html::encode_path(&controller_id));
    if let Some(tag_id) = payload.get("tag_id").and_then(|v| v.parse::<i64>().ok()) {
        let _ = hawkbit::send_empty(
            &state,
            &session,
            Method::POST,
            &format!(
                "targettags/{tag_id}/assigned/{}",
                html::encode_path(&controller_id)
            ),
        )
        .await;
    }
    Redirect::to(&next).into_response()
}

async fn remove_target_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((controller_id, tag_id)): AxumPath<(String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let _ = hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "targettags/{}/assigned/{}",
            html::encode_path(&tag_id),
            html::encode_path(&controller_id)
        ),
    )
    .await;
    Redirect::to(&format!("/targets/{}", html::encode_path(&controller_id))).into_response()
}

async fn toggle_auto_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((controller_id, toggle)): AxumPath<(String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/targets/{}", html::encode_path(&controller_id));
    if !matches!(toggle.as_str(), "activate" | "deactivate") {
        return Redirect::to(&next).into_response();
    }
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "targets",
            resource_id: &controller_id,
            action: "auto-confirm",
            request_summary: format!("auto-confirm {toggle}"),
        },
    )
    .await
    .ok();
    let result = if toggle == "activate" {
        hawkbit::send_json(
            &state,
            &session,
            Method::POST,
            &format!(
                "targets/{}/autoConfirm/activate",
                html::encode_path(&controller_id)
            ),
            json!({ "initiator": session.username }),
        )
        .await
        .map(|_| ())
    } else {
        hawkbit::send_empty(
            &state,
            &session,
            Method::POST,
            &format!(
                "targets/{}/autoConfirm/deactivate",
                html::encode_path(&controller_id)
            ),
        )
        .await
        .map(|_| ())
    };
    match result {
        Ok(()) => finish_log(&state, log.as_ref(), "success", &toggle, "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn set_target_type(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(controller_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let path = format!("targets/{}/targettype", html::encode_path(&controller_id));
    let result = match payload.get("type_id").and_then(|v| v.parse::<i64>().ok()) {
        Some(type_id) => hawkbit::send_json(
            &state,
            &session,
            Method::POST,
            &path,
            json!({ "id": type_id }),
        )
        .await
        .map(|_| ()),
        // An empty picker clears the type.
        None => hawkbit::send_empty(&state, &session, Method::DELETE, &path)
            .await
            .map(|_| ()),
    };
    if let Err(error) = result {
        tracing::warn!(%controller_id, error = %error.message, "target type change failed");
    }
    Redirect::to(&format!("/targets/{}", html::encode_path(&controller_id))).into_response()
}

async fn cancel_target_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((controller_id, action_id)): AxumPath<(String, String)>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let force = payload.get("force").map(String::as_str) == Some("true");
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "targets",
            resource_id: &controller_id,
            action: if force { "force-cancel" } else { "cancel" },
            request_summary: format!("cancel action {action_id} (force={force})"),
        },
    )
    .await
    .ok();
    let path = format!(
        "targets/{}/actions/{}{}",
        html::encode_path(&controller_id),
        html::encode_path(&action_id),
        if force { "?force=true" } else { "" }
    );
    match hawkbit::send_empty(&state, &session, Method::DELETE, &path).await {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "canceled", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&format!("/targets/{}", html::encode_path(&controller_id))).into_response()
}

async fn add_ds_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(ds_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    if let Some(tag_id) = payload.get("tag_id").and_then(|v| v.parse::<i64>().ok()) {
        let _ = hawkbit::send_empty(
            &state,
            &session,
            Method::POST,
            &format!(
                "distributionsettags/{tag_id}/assigned/{}",
                html::encode_path(&ds_id)
            ),
        )
        .await;
    }
    Redirect::to(&format!("/distribution-sets/{}", html::encode_path(&ds_id))).into_response()
}

async fn remove_ds_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((ds_id, tag_id)): AxumPath<(String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let _ = hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "distributionsettags/{}/assigned/{}",
            html::encode_path(&tag_id),
            html::encode_path(&ds_id)
        ),
    )
    .await;
    Redirect::to(&format!("/distribution-sets/{}", html::encode_path(&ds_id))).into_response()
}

fn ds_type_module_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "mandatory" => Some("mandatorymoduletypes"),
        "optional" => Some("optionalmoduletypes"),
        _ => None,
    }
}

async fn add_ds_type_module(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(type_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/distribution-set-types/{}", html::encode_path(&type_id));
    let kind = payload
        .get("kind")
        .and_then(|k| ds_type_module_kind(k))
        .unwrap_or("optionalmoduletypes");
    if let Some(sm_type_id) = payload
        .get("sm_type_id")
        .and_then(|v| v.parse::<i64>().ok())
    {
        let _ = hawkbit::send_json(
            &state,
            &session,
            Method::POST,
            &format!(
                "distributionsettypes/{}/{kind}",
                html::encode_path(&type_id)
            ),
            json!([{ "id": sm_type_id }]),
        )
        .await;
    }
    Redirect::to(&next).into_response()
}

async fn remove_ds_type_module(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((type_id, kind, sm_type_id)): AxumPath<(String, String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/distribution-set-types/{}", html::encode_path(&type_id));
    let Some(kind) = ds_type_module_kind(&kind) else {
        return Redirect::to(&next).into_response();
    };
    let _ = hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "distributionsettypes/{}/{kind}/{}",
            html::encode_path(&type_id),
            html::encode_path(&sm_type_id)
        ),
    )
    .await;
    Redirect::to(&next).into_response()
}

async fn assign_ds_to_target(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(controller_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/targets/{}", html::encode_path(&controller_id));
    let Some(ds_id) = payload.get("ds_id").and_then(|v| v.parse::<i64>().ok()) else {
        return Redirect::to(&next).into_response();
    };
    let action_type = payload
        .get("type")
        .cloned()
        .unwrap_or_else(|| "forced".to_string());
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "targets",
            resource_id: &controller_id,
            action: "assign-ds",
            request_summary: format!("assign DS {ds_id} ({action_type})"),
        },
    )
    .await
    .ok();
    match hawkbit::send_json(
        &state,
        &session,
        Method::POST,
        &format!("targets/{}/assignedDS", html::encode_path(&controller_id)),
        serde_json::json!({ "id": ds_id, "type": action_type }),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "assigned", "").await,
        Err(error) => {
            finish_log(&state, log.as_ref(), "error", "", &error.message).await;
            // A refusal must not look like a success: going back to the
            // target page would show it unchanged and say nothing.
            return message_page(
                &state,
                &session,
                "Targets",
                resources::by_id("targets").as_ref(),
                format!("Assign stopped: {}", error.message),
                true,
            )
            .await;
        }
    }
    Redirect::to(&next).into_response()
}

// ---------------------------------------------------------------------------
// Metadata, target groups, compatible types, auto-assignment
// ---------------------------------------------------------------------------

/// Objects that carry metadata. The console path segment maps straight onto
/// the mgmt endpoint, but the set is closed on purpose: `/{anything}/metadata`
/// must not become a way to poke arbitrary upstream paths.
fn metadata_resource(resource_id: &str) -> Option<Resource> {
    matches!(
        resource_id,
        "targets" | "distribution-sets" | "software-modules"
    )
    .then(|| resources::by_id(resource_id))
    .flatten()
}

async fn save_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id)): AxumPath<(String, String)>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(resource) = metadata_resource(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/{}/{}", resource.id, html::encode_path(&id));
    let Some(key) = payload.get("key").filter(|key| !key.trim().is_empty()) else {
        return Redirect::to(&next).into_response();
    };
    let mut entry = json!({
        "key": key,
        "value": payload.get("value").cloned().unwrap_or_default(),
    });
    if resource.id == "software-modules" {
        entry["targetVisible"] = json!(payload.contains_key("targetVisible"));
    }
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: &resource.id,
            resource_id: &id,
            action: "metadata-save",
            request_summary: format!("set metadata {key}"),
        },
    )
    .await
    .ok();
    // The server upserts, so one call both adds and edits.
    match hawkbit::send_json(
        &state,
        &session,
        Method::POST,
        &format!("{}/{}/metadata", resource.endpoint, html::encode_path(&id)),
        json!([entry]),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "saved", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn delete_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((resource_id, id)): AxumPath<(String, String)>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(resource) = metadata_resource(&resource_id) else {
        return not_found().await;
    };
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/{}/{}", resource.id, html::encode_path(&id));
    // The key arrives in the body: keys hold slashes and percent signs, and
    // a path segment would mangle them.
    let Some(key) = payload.get("key").filter(|key| !key.is_empty()) else {
        return Redirect::to(&next).into_response();
    };
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: &resource.id,
            resource_id: &id,
            action: "metadata-delete",
            request_summary: format!("delete metadata {key}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "{}/{}/metadata/{}",
            resource.endpoint,
            html::encode_path(&id),
            html::encode_path(key)
        ),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "deleted", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn set_target_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(controller_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/targets/{}", html::encode_path(&controller_id));
    let Some(group) = payload
        .get("group")
        .filter(|group| !group.trim().is_empty())
    else {
        return Redirect::to(&next).into_response();
    };
    let outcome = assign_group(
        &state,
        &session,
        Some(group),
        std::slice::from_ref(&controller_id),
    )
    .await;
    log_group_change(&state, &session, &controller_id, group, outcome).await;
    Redirect::to(&next).into_response()
}

async fn clear_target_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(controller_id): AxumPath<String>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let outcome = assign_group(&state, &session, None, std::slice::from_ref(&controller_id)).await;
    log_group_change(&state, &session, &controller_id, "", outcome).await;
    Redirect::to(&format!("/targets/{}", html::encode_path(&controller_id))).into_response()
}

/// `PUT`/`DELETE /targetgroups/assigned` both take the controller ids in the
/// body, which is exactly what a row selection hands over.
async fn assign_group(
    state: &AppState,
    session: &session::Session,
    group: Option<&str>,
    ids: &[String],
) -> Result<(), String> {
    let (method, path) = match group {
        Some(group) => (
            Method::PUT,
            format!(
                "targetgroups/assigned?group={}",
                html::encode_component(group)
            ),
        ),
        None => (Method::DELETE, "targetgroups/assigned".to_string()),
    };
    hawkbit::send_json(state, session, method, &path, json!(ids))
        .await
        .map(|_| ())
        .map_err(|error| error.message)
}

async fn log_group_change(
    state: &AppState,
    session: &session::Session,
    controller_id: &str,
    group: &str,
    outcome: Result<(), String>,
) {
    let log = logs::record_started(
        state,
        session,
        ActionLogDraft {
            resource: "targets",
            resource_id: controller_id,
            action: "set-group",
            request_summary: if group.is_empty() {
                "clear group".to_string()
            } else {
                format!("set group {group}")
            },
        },
    )
    .await
    .ok();
    match outcome {
        Ok(()) => finish_log(state, log.as_ref(), "success", "done", "").await,
        Err(error) => finish_log(state, log.as_ref(), "error", "", &error).await,
    }
}

async fn add_compatible_ds_type(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(type_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/target-types/{}", html::encode_path(&type_id));
    let Some(ds_type_id) = payload
        .get("ds_type_id")
        .and_then(|v| v.parse::<i64>().ok())
    else {
        return Redirect::to(&next).into_response();
    };
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "target-types",
            resource_id: &type_id,
            action: "add-ds-type",
            request_summary: format!("allow distribution set type {ds_type_id}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_json(
        &state,
        &session,
        Method::POST,
        &format!(
            "targettypes/{}/compatibledistributionsettypes",
            html::encode_path(&type_id)
        ),
        json!([{ "id": ds_type_id }]),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "added", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn remove_compatible_ds_type(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((type_id, ds_type_id)): AxumPath<(String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/target-types/{}", html::encode_path(&type_id));
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "target-types",
            resource_id: &type_id,
            action: "remove-ds-type",
            request_summary: format!("disallow distribution set type {ds_type_id}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "targettypes/{}/compatibledistributionsettypes/{}",
            html::encode_path(&type_id),
            html::encode_path(&ds_type_id)
        ),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "removed", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn set_auto_assign(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(filter_id): AxumPath<String>,
    Form(payload): Form<HashMap<String, String>>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/target-filters/{}", html::encode_path(&filter_id));
    let Some(ds_id) = payload.get("ds_id").and_then(|v| v.parse::<i64>().ok()) else {
        return Redirect::to(&next).into_response();
    };
    let mut body = json!({
        "id": ds_id,
        "type": payload
            .get("type")
            .cloned()
            .unwrap_or_else(|| "forced".to_string()),
        "confirmationRequired": payload.contains_key("confirmationRequired"),
    });
    if let Some(weight) = payload.get("weight").and_then(|v| v.parse::<i64>().ok()) {
        body["weight"] = json!(weight);
    }
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "target-filters",
            resource_id: &filter_id,
            action: "auto-assign",
            request_summary: format!("auto-assign DS {ds_id}"),
        },
    )
    .await
    .ok();
    match hawkbit::send_json(
        &state,
        &session,
        Method::POST,
        &format!(
            "targetfilters/{}/autoAssignDS",
            html::encode_path(&filter_id)
        ),
        body,
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "assigned", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn clear_auto_assign(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(filter_id): AxumPath<String>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let next = format!("/target-filters/{}", html::encode_path(&filter_id));
    let log = logs::record_started(
        &state,
        &session,
        ActionLogDraft {
            resource: "target-filters",
            resource_id: &filter_id,
            action: "auto-assign-clear",
            request_summary: "stop auto-assigning".to_string(),
        },
    )
    .await
    .ok();
    match hawkbit::send_empty(
        &state,
        &session,
        Method::DELETE,
        &format!(
            "targetfilters/{}/autoAssignDS",
            html::encode_path(&filter_id)
        ),
    )
    .await
    {
        Ok(_) => finish_log(&state, log.as_ref(), "success", "cleared", "").await,
        Err(error) => finish_log(&state, log.as_ref(), "error", "", &error.message).await,
    }
    Redirect::to(&next).into_response()
}

async fn download_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((sm_id, artifact_id)): AxumPath<(String, String)>,
) -> Response {
    let Some(session) = session::current(&state, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    let path = format!(
        "softwaremodules/{}/artifacts/{}/download",
        html::encode_path(&sm_id),
        html::encode_path(&artifact_id)
    );
    match hawkbit::stream(&state, &session, &path, headers.get(http::header::RANGE)).await {
        Ok(response) => response,
        Err(error) => {
            message_page(
                &state,
                &session,
                "Download failed",
                resources::by_id("software-modules").as_ref(),
                error.message,
                true,
            )
            .await
        }
    }
}
