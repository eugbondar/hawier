//! View models and page templates.
//!
//! Every piece of markup lives in `apps/ui/templates`; this module only
//! projects JSON payloads and application state into typed models that the
//! templates lay out. The one exception is [`icon`], which returns a static
//! SVG fragment — a shape, not a page structure.

use askama::Template;
use axum::response::{Html, IntoResponse, Response};
use serde_json::Value;

use crate::{
    help::{self, Block, Span},
    html::{
        encode_component, encode_path, human_stamp, is_time_field, read_path, render_plain_value,
    },
    resources::{Group, Resource},
};

/// Render a template into a response. A template that fails to render is a
/// bug in the template, not a user-visible condition — say so plainly
/// instead of serving half a page.
pub fn respond<T: Template>(template: &T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(error) => {
            tracing::error!(%error, "template render failed");
            // 500, not 200: the audit crawls every page and asserts 2xx, so a
            // broken template has to fail the build. Answering 200 with an
            // apology let a forgotten `.iter()` inside a macro pass as green.
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("template render failed: {error}")),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Cells — the one vocabulary for "a value on a page"
// ---------------------------------------------------------------------------

pub enum Cell {
    Text(String),
    Muted(String),
    Badge {
        class: String,
        text: String,
        /// Key of the glossary entry for this word — every status in the
        /// console explains itself.
        help: String,
    },
    Time {
        epoch: i64,
        human: String,
    },
    TimeFull {
        epoch: i64,
        human: String,
    },
    Overdue {
        epoch: i64,
        human: String,
    },
    Link {
        href: String,
        label: String,
    },
}

impl Cell {
    pub fn badge(class: &str, text: &str) -> Self {
        Cell::Badge {
            class: class.to_string(),
            text: text.to_string(),
            help: help::status_key(text),
        }
    }

    pub fn time(epoch: i64) -> Self {
        Cell::Time {
            epoch,
            human: human_stamp(epoch),
        }
    }

    pub fn time_full(epoch: i64) -> Self {
        Cell::TimeFull {
            epoch,
            human: human_stamp(epoch),
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Cell::Text(value.into())
    }
}

/// `<name>Name` fields whose sibling `<name>` holds the id of a resource.
const REFERENCE_FIELDS: [(&str, &str, &str); 3] = [
    (
        "assignedDistributionSetName",
        "assignedDistributionSet",
        "distribution-sets",
    ),
    (
        "installedDistributionSetName",
        "installedDistributionSet",
        "distribution-sets",
    ),
    ("rolloutName", "rollout", "rollouts"),
];

/// Render one field by name, given the row it belongs to. A cell that names
/// another object links there — a list cell that cannot be opened is a dead
/// end. `full` spells the raw epoch out beside a timestamp (object pages).
pub fn field_cell(name: &str, value: Option<&Value>, row: Option<&Value>, full: bool) -> Cell {
    if let Some(link) = reference_cell(name, value, row) {
        return link;
    }
    if let Some(poll) = poll_state_cell(name, value) {
        return poll;
    }
    if let Some(poll) = poll_status_cell(name, value) {
        return poll;
    }
    match time_millis(name, value) {
        Some(millis) if full => Cell::time_full(millis),
        Some(millis) => Cell::time(millis),
        None => value_cell(value),
    }
}

fn reference_cell(name: &str, value: Option<&Value>, row: Option<&Value>) -> Option<Cell> {
    let (_, id_field, resource) = REFERENCE_FIELDS
        .iter()
        .find(|(label, _, _)| *label == name)?;
    let label = value?.as_str().filter(|label| !label.is_empty())?;
    let id = row?.get(id_field).filter(|id| !id.is_null())?;
    Some(Cell::Link {
        href: format!("/{resource}/{}", encode_path(&render_plain_value(id))),
        label: label.to_string(),
    })
}

/// hawkBit's `pollStatus` is a nested object; the generic renderer used to
/// collapse it to the literal word "object".
fn poll_status_cell(name: &str, value: Option<&Value>) -> Option<Cell> {
    if name != "pollStatus" {
        return None;
    }
    let value = value?;
    let last = value.get("lastRequestAt").and_then(Value::as_i64);
    let overdue = value
        .get("overdue")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(match last {
        Some(last) if overdue => Cell::Overdue {
            epoch: last,
            human: human_stamp(last),
        },
        Some(last) => Cell::time(last),
        None => Cell::Muted("never polled".to_string()),
    })
}

/// hawier's scalar liveness field. The nested `pollStatus` beside it on the
/// object page is the same fact — the server derives both from one SQL
/// expression — so the two cannot disagree; this one is the list column.
///
/// Deliberately not `Cell::Overdue`: that prints a pill *and* the timestamp,
/// and in a list the very next column is that timestamp. Here the badge alone
/// is the information.
fn poll_state_cell(name: &str, value: Option<&Value>) -> Option<Cell> {
    if name != "pollState" {
        return None;
    }
    Some(match value?.as_str()? {
        "responding" => Cell::badge("ok", "responding"),
        "overdue" => Cell::badge("error", "overdue"),
        // The wire word is a token; this is the sentence, and it is the same
        // sentence `poll_status_cell` uses on the card.
        "never" => Cell::Muted("never polled".to_string()),
        other => Cell::text(other.to_string()),
    })
}

/// Epoch millis if this field is a timestamp with a usable value.
fn time_millis(name: &str, value: Option<&Value>) -> Option<i64> {
    if !is_time_field(name) {
        return None;
    }
    match value?.as_i64()? {
        0 => None,
        millis => Some(millis),
    }
}

pub fn value_cell(value: Option<&Value>) -> Cell {
    let Some(value) = value else {
        return Cell::Muted("empty".to_string());
    };
    match value {
        // An absent value and an explicit null are the same fact to an
        // operator; "null" is the wire's word, not theirs. A column that is
        // missing from the payload entirely is caught by the audit against
        // the JSON, not by making the cell read differently here.
        Value::Null => Cell::Muted("empty".to_string()),
        Value::Bool(true) => Cell::text("yes"),
        Value::Bool(false) => Cell::text("no"),
        Value::Number(number) => Cell::text(number.to_string()),
        Value::String(text) if is_status(text) => Cell::badge(status_class(text), text),
        Value::String(text) => Cell::text(text.clone()),
        Value::Array(items) => Cell::text(format!("{} items", items.len())),
        Value::Object(_) => Cell::text(render_plain_value(value)),
    }
}

/// A JSON string that names a state, and therefore reads as a badge rather
/// than as text. The vocabulary lives in one table — see [`help::status`].
fn is_status(value: &str) -> bool {
    help::status(value).is_some()
}

pub fn status_class(value: &str) -> &'static str {
    help::status(value).map_or("neutral", |found| found.class)
}

/// One `key → value` row on an object page: the human rendering plus the
/// value exactly as the API sent it (what a FIQL filter takes).
pub struct Field {
    pub key: String,
    pub cell: Cell,
    pub raw: String,
    /// Where the manual explains this field.
    pub help: String,
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

pub struct Layout {
    pub title: String,
    /// Key of the manual section that answers for this page as a whole — the
    /// last resort when a help-mode click lands on nothing more specific.
    pub help: String,
    pub search_query: String,
    pub groups: Vec<NavGroup>,
    pub session: Option<Chip>,
}

pub struct Chip {
    pub tenant: String,
    pub username: String,
}

pub struct NavGroup {
    pub help: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub open: bool,
    pub active: bool,
    pub is_admin: bool,
    pub resources: Vec<NavResource>,
}

pub struct NavResource {
    pub help: String,
    pub id: String,
    pub label: String,
    pub icon: &'static str,
    pub active: bool,
    pub views: Vec<NavView>,
    pub saved: Vec<NavView>,
}

pub struct NavView {
    pub help: String,
    pub href: String,
    pub edit_href: String,
    pub label: String,
    pub active: bool,
}

pub fn group_label(group: Group) -> &'static str {
    match group {
        Group::Inventory => "Inventory",
        Group::Deployment => "Deployment",
        Group::Repository => "Repository",
        Group::Admin => "Admin",
    }
}

pub fn group_key(group: Group) -> &'static str {
    match group {
        Group::Inventory => "inventory",
        Group::Deployment => "deployment",
        Group::Repository => "repository",
        Group::Admin => "admin",
    }
}

/// Every resource gets its own mark: in the collapsed rail the icon is all
/// there is to tell them apart.
pub fn resource_icon(resource: &Resource) -> &'static str {
    // A declared surface brings its own name; built-ins are known by id.
    if let Some(name) = resource.icon.as_deref() {
        return icon_name(name);
    }
    match resource.id.as_str() {
        "targets" => "targets",
        "rollouts" => "rollouts",
        "distribution-sets" => "distribution",
        "target-filters" => "filters",
        "software-modules" => "modules",
        "actions" => "actions",
        "incidents" => "incidents",
        "action-log" => "logs",
        "retention-rules" => "retention",
        "roles" => "role",
        "grants" => "grant",
        "webhooks" => "webhook",
        "webhook-deliveries" => "delivery",
        "target-groups" => "group",
        "target-tags" | "distribution-set-tags" => "tag",
        "target-types" | "distribution-set-types" | "software-module-types" => "type",
        _ => "resource",
    }
}

/// Resolve a declared icon name against the console's own set, so a
/// declaration cannot ask for a shape that does not exist.
fn icon_name(name: &str) -> &'static str {
    match name {
        "targets" => "targets",
        "rollouts" => "rollouts",
        "distribution" => "distribution",
        "filters" => "filters",
        "modules" => "modules",
        "actions" => "actions",
        "incidents" => "incidents",
        "logs" => "logs",
        "group" => "group",
        "tag" => "tag",
        "type" => "type",
        "webhook" => "webhook",
        "delivery" => "delivery",
        "retention" => "retention",
        // Reserved: nothing declares a surface today (ext-console
        // contributes views instead), but the shape is here so a
        // deployment that declares one has a mark of its own.
        "health" => "health",
        _ => "resource",
    }
}

/// The console's icon set. A shape, not page structure — the one HTML-ish
/// helper the doctrine leaves in Rust.
pub fn icon(name: &str) -> &'static str {
    match name {
        "burger" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 3h12v2H2V3zm0 4h12v2H2V7zm0 4h12v2H2v-2z\"/></svg>"
        }
        "logout" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M7 2v2H4v8h3v2H2V2h5zm3.6 2.6L14 8l-3.4 3.4-1.4-1.4L10.2 9H6V7h4.2L9.2 6l1.4-1.4z\"/></svg>"
        }
        "chevron" => {
            "<svg class=\"tree-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M6 3l5 5-5 5V3z\"/></svg>"
        }
        "overview" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 3h5v4H2V3zm7 0h5v4H9V3zM2 9h5v4H2V9zm7 0h5v4H9V9z\"/></svg>"
        }
        "targets" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 1a7 7 0 107 7h-2a5 5 0 11-5-5V1zm1 2v6h6A6 6 0 009 3z\"/></svg>"
        }
        "rollouts" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 3h8v2H2V3zm0 4h12v2H2V7zm0 4h8v2H2v-2zm10-8l3 3-3 3V3z\"/></svg>"
        }
        "distribution" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 1l6 3v8l-6 3-6-3V4l6-3zm0 2.2L4.4 5 8 6.8 11.6 5 8 3.2zM4 6.6v4.3l3 1.5V8.1L4 6.6zm5 5.8l3-1.5V6.6L9 8.1v4.3z\"/></svg>"
        }
        "filters" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 3h12L9 9v4l-2 1V9L2 3z\"/></svg>"
        }
        "modules" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M3 2h10v3H3V2zm0 5h10v3H3V7zm0 5h10v2H3v-2z\"/></svg>"
        }
        "settings" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M6.8 1h2.4l.3 1.7 1.4.6 1.4-.9 1.7 1.7-.9 1.4.6 1.4 1.7.3v2.4l-1.7.3-.6 1.4.9 1.4-1.7 1.7-1.4-.9-1.4.6-.3 1.7H6.8l-.3-1.7-1.4-.6-1.4.9-1.7-1.7.9-1.4-.6-1.4L.6 9.2V6.8l1.7-.3.6-1.4-.9-1.4 1.7-1.7 1.4.9 1.4-.6L6.8 1zM8 5.6a2.4 2.4 0 100 4.8 2.4 2.4 0 000-4.8z\"/></svg>"
        }
        "logs" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M3 2h10v12H3V2zm2 3v1h6V5H5zm0 3v1h6V8H5zm0 3v1h4v-1H5z\"/></svg>"
        }
        "health" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M1 8h3l2-5 3 10 2-5h4v2h-2.8l-3.2 8-3-10-1.2 3H1V8z\"/></svg>"
        }
        "role" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 1l6 2.5v4c0 3.4-2.4 6.4-6 7.5-3.6-1.1-6-4.1-6-7.5v-4L8 1zm0 4a2 2 0 100 4 2 2 0 000-4zm-3 7c.6-1.4 1.7-2 3-2s2.4.6 3 2H5z\"/></svg>"
        }
        "grant" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M5.5 2a3 3 0 013 3c0 1.2-.7 2.2-1.7 2.7C8.6 8.3 9.8 9.7 10 11.5H1c.2-1.8 1.4-3.2 3.2-3.8A3 3 0 015.5 2zM12 6l3 3-3 3-1.1-1.1 1.1-1.1H9V8.2h3L10.9 7 12 6z\"/></svg>"
        }
        "retention" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M6 1h4v1h3v2H3V2h3V1zM4 5h8l-.7 9H4.7L4 5zm2.2 2v5h1V7h-1zm2.6 0v5h1V7h-1z\"/></svg>"
        }
        "webhook" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 1a3 3 0 00-3 3h2a1 1 0 112 0c0 .6-.3.9-.9 1.4-.7.6-1.6 1.3-1.6 2.8V9h2v-.8c0-.6.3-.9.9-1.4C10.1 6.2 11 5.5 11 4a3 3 0 00-3-3zM4 11v2h8v-2H9.5v-1h-3v1H4z\"/></svg>"
        }
        "delivery" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 3h12v3H2V3zm0 4h5v2H2V7zm0 3h5v3H2v-3zm7.8-.6l1.4-1.4L14 10.8 11.2 14l-1.4-1.4 1-1H8v-2h2.8l-1-1.2z\"/></svg>"
        }
        "pencil" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M11.6 1.6l2.8 2.8-1.4 1.4-2.8-2.8 1.4-1.4zM9.5 3.7l2.8 2.8L5.8 13H3v-2.8l6.5-6.5z\"/></svg>"
        }
        "incidents" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 1l7 13H1L8 1zm-1 5v4h2V6H7zm0 5v2h2v-2H7z\"/></svg>"
        }
        "actions" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M9 1L3 9h4l-1 6 6-8H8l1-6z\"/></svg>"
        }
        "group" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 2h5v3H2V2zm0 5h5v3H2V7zm0 5h5v2H2v-2zm7-8h5v3H9V4zm0 5h5v3H9V9z\"/></svg>"
        }
        "tag" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 2h6l6 6-6 6-6-6V2zm2.5 1.5a1.2 1.2 0 100 2.4 1.2 1.2 0 000-2.4z\"/></svg>"
        }
        "type" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M2 2h12v3h-2V4H9v8h2v2H5v-2h2V4H4v1H2V2z\"/></svg>"
        }
        "view" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 3c4 0 7 5 7 5s-3 5-7 5-7-5-7-5 3-5 7-5zm0 2a3 3 0 100 6 3 3 0 000-6z\"/></svg>"
        }
        "user-view" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 2a3 3 0 110 6 3 3 0 010-6zm-5 12a5 5 0 0110 0H3z\"/></svg>"
        }
        "help" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M8 1a7 7 0 100 14A7 7 0 008 1zm0 2.4c1.6 0 2.8 1 2.8 2.4 0 1.1-.6 1.7-1.5 2.3-.5.3-.6.5-.6 1v.4H7.3v-.6c0-.9.3-1.4 1.1-1.9.6-.4.9-.7.9-1.2 0-.6-.5-1-1.3-1-.8 0-1.3.4-1.4 1.1H5.2c.1-1.5 1.2-2.5 2.8-2.5zM8 10.9a1 1 0 110 2 1 1 0 010-2z\"/></svg>"
        }
        "close" => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M4 2.6L8 6.6l4-4L13.4 4l-4 4 4 4-1.4 1.4-4-4-4 4L2.6 12l4-4-4-4L4 2.6z\"/></svg>"
        }
        _ => {
            "<svg class=\"item-icon\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path fill=\"currentColor\" d=\"M3 3h10v10H3V3z\"/></svg>"
        }
    }
}

// ---------------------------------------------------------------------------
// Paging
// ---------------------------------------------------------------------------

pub struct Hidden {
    pub name: String,
    pub value: String,
}

pub struct PagerJump {
    pub action: String,
    pub fields: Vec<Hidden>,
}

pub struct Pager {
    pub first_row: usize,
    pub last_row: usize,
    pub total: usize,
    pub page: usize,
    pub last_page: usize,
    pub has_prev: bool,
    pub has_next: bool,
    pub first_href: String,
    pub prev_href: String,
    pub next_href: String,
    pub last_href: String,
    pub jump: Option<PagerJump>,
}

impl Pager {
    pub fn new(
        page: usize,
        limit: usize,
        total: usize,
        shown: usize,
        href: &dyn Fn(usize) -> String,
    ) -> Self {
        let last_page = if total == 0 { 1 } else { total.div_ceil(limit) };
        let first_row = if shown == 0 {
            0
        } else {
            (page - 1) * limit + 1
        };
        Self {
            first_row,
            last_row: (page - 1) * limit + shown,
            total,
            page,
            last_page,
            has_prev: page > 1,
            has_next: page < last_page,
            first_href: href(1),
            prev_href: href(page.saturating_sub(1).max(1)),
            next_href: href(page + 1),
            last_href: href(last_page),
            jump: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pickers — a searchable field instead of a server-filled dropdown
// ---------------------------------------------------------------------------

pub struct PickerSpec {
    /// Name of the form field that carries the chosen id.
    pub field: &'static str,
    /// Console resource the typeahead searches (`/pick/{resource}`).
    pub resource: &'static str,
    pub placeholder: &'static str,
    pub value: String,
    pub label: String,
    /// Non-empty when the input lives outside the form it belongs to.
    pub form_id: &'static str,
    /// Typing keeps its own value instead of clearing the choice.
    pub free: bool,
}

impl PickerSpec {
    pub fn new(field: &'static str, resource: &'static str, placeholder: &'static str) -> Self {
        Self {
            field,
            resource,
            placeholder,
            value: String::new(),
            label: String::new(),
            form_id: "",
            free: false,
        }
    }

    pub fn in_form(mut self, form_id: &'static str) -> Self {
        self.form_id = form_id;
        self
    }

    /// The field suggests existing values but accepts anything typed. Used
    /// where the value is free text with a habit — a target group is a path
    /// the operator may well be inventing right now.
    pub fn free_text(mut self) -> Self {
        self.free = true;
        self
    }

    pub fn with_current(mut self, value: String, label: String) -> Self {
        self.value = value;
        self.label = label;
        self
    }
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "message.html")]
pub struct MessagePage {
    pub layout: Layout,
    pub message: String,
    pub danger: bool,
}

pub struct StatCard {
    pub href: String,
    pub label: String,
    pub count: String,
}

pub struct CountRow {
    pub id: String,
    pub label: String,
    pub count: String,
    pub group: &'static str,
    pub new_href: Option<String>,
}

pub struct RolloutSummary {
    pub href: String,
    pub name: String,
    pub status: String,
    pub finished: i64,
    pub total: i64,
}

pub struct IncidentSummary {
    pub number: String,
    pub status: String,
    pub route: String,
    pub message: String,
}

#[derive(Template)]
#[template(path = "overview.html")]
pub struct OverviewPage {
    pub layout: Layout,
    pub status_cards: Vec<StatCard>,
    pub counts: Vec<CountRow>,
    pub rollouts: Vec<RolloutSummary>,
    pub incidents: Vec<IncidentSummary>,
    pub console_noise: u64,
}

pub struct ColumnPick {
    pub name: String,
    pub checked: bool,
}

pub struct Header {
    pub label: String,
    pub href: String,
    pub help: String,
}

pub struct Row {
    pub id: String,
    pub href: String,
    pub cells: Vec<Cell>,
}

pub struct BulkItem {
    pub id: &'static str,
    pub label: &'static str,
    pub danger: bool,
    pub help: String,
}

/// Operations that share one picker are grouped so the form carries a single
/// input per kind — two `tag_id` fields would make "untag" ambiguous.
pub struct BulkGroup {
    pub picker: Option<PickerSpec>,
    pub actions: Vec<BulkItem>,
}

#[derive(Template)]
#[template(path = "list.html")]
pub struct ListPage {
    pub layout: Layout,
    pub resource_id: String,
    pub label: String,
    pub new_href: Option<String>,
    pub view: String,
    pub q: String,
    pub sort: String,
    pub limit: usize,
    pub columns_csv: String,
    pub column_picks: Vec<ColumnPick>,
    pub headers: Vec<Header>,
    pub rows: Vec<Row>,
    pub colspan: usize,
    pub pager: Pager,
    pub bulk: Vec<BulkGroup>,
    pub error: Option<String>,
}

pub struct StateAction {
    pub action: String,
    pub label: String,
    pub confirm: String,
    pub help: String,
}

#[derive(Template)]
#[template(path = "detail.html")]
pub struct DetailPage {
    pub layout: Layout,
    pub label: String,
    pub id: String,
    pub edit_href: Option<String>,
    pub delete_action: String,
    pub state_actions: Vec<StateAction>,
    pub autorefresh: bool,
    pub fields: Vec<Field>,
    pub extra: DetailExtra,
}

// One page holds one of these, so the size spread between variants costs a
// few hundred bytes per request and buys a flat `match` in the template.
#[allow(clippy::large_enum_variant)]
pub enum DetailExtra {
    None,
    Rollout(RolloutPanel),
    SoftwareModule(SmPanel),
    DistributionSet(DsPanel),
    Target(TargetPanel),
    Incident(IncidentPanel),
    DsType(DsTypePanel),
    TargetType(TargetTypePanel),
    TargetFilter(AutoAssignPanel),
}

/// Free-form `key → value` pairs an operator can attach to an object.
/// Values are rendered as plain text on purpose: a value of `error` must not
/// turn into a status pill and read as the device's state.
pub struct MetadataRow {
    pub key: String,
    pub value: String,
    pub target_visible: bool,
}

pub struct MetadataPanel {
    pub rows: Vec<MetadataRow>,
    pub save_action: String,
    pub delete_action: String,
    /// Only software modules carry the `targetVisible` flag.
    pub shows_target_visible: bool,
    pub pager: Pager,
}

pub struct SmPanel {
    pub artifacts: ArtifactsPanel,
    pub metadata: MetadataPanel,
}

pub struct TargetTypePanel {
    pub compatible: Vec<ChipRow>,
    pub add_action: String,
    pub picker: PickerSpec,
}

/// Auto-assignment on a target filter: the rule that hands a distribution
/// set to every device the filter matches, from now on.
pub struct AutoAssignPanel {
    pub set_action: String,
    pub clear_action: String,
    pub picker: PickerSpec,
    /// Set only while a distribution set is assigned — the weight and the
    /// confirmation flag survive a clear on the server, so showing them
    /// afterwards would claim they still apply.
    pub current: Option<AutoAssignCurrent>,
}

pub struct AutoAssignCurrent {
    pub label: String,
    pub href: String,
    pub action_type: String,
    pub weight: i64,
    pub confirmation_required: bool,
}

pub struct ProgressBar {
    pub label: &'static str,
    pub class: &'static str,
    pub percent: i64,
    pub count: i64,
}

pub struct GroupRow {
    pub href: String,
    pub name: String,
    pub status: String,
    pub total: i64,
    pub success: String,
    pub error: String,
}

pub struct RolloutPanel {
    pub bars: Vec<ProgressBar>,
    pub groups: Vec<GroupRow>,
}

pub struct ArtifactRow {
    pub download_href: String,
    pub name: String,
    pub size: String,
    pub sha1: String,
    pub sha1_short: String,
    pub delete_action: String,
}

pub struct ArtifactsPanel {
    pub upload_action: String,
    pub next: String,
    pub rows: Vec<ArtifactRow>,
}

pub struct ModuleNode {
    pub name: String,
    pub version: String,
    pub kind: String,
    pub detach_action: String,
    pub artifacts: ArtifactsPanel,
}

pub struct ChipRow {
    pub label: String,
    pub colour: String,
    pub remove_action: String,
}

pub struct TagsPanel {
    pub chips: Vec<ChipRow>,
    pub add_action: String,
    pub picker: PickerSpec,
    pub create_href: &'static str,
}

pub struct DsPanel {
    /// "Who has this set" — each card counts with exactly the query it links
    /// to, so the number and the list can never disagree.
    pub usage: Vec<StatCard>,
    pub metadata: MetadataPanel,
    pub modules: Vec<ModuleNode>,
    pub attach_action: String,
    pub attach_picker: PickerSpec,
    pub next: String,
    pub create_module_href: String,
    pub rollout_href: String,
    pub tags: TagsPanel,
}

pub struct ActionRow {
    pub href: String,
    pub id: String,
    pub detail: String,
    pub kind: String,
    pub status: String,
    pub cancel_action: Option<String>,
}

pub struct TargetPanel {
    pub metadata: MetadataPanel,
    /// Free-text hierarchical path (`eu/west`). Not a picker's strict
    /// choice: an operator must be able to type a path that does not exist
    /// yet, so the field takes suggestions but accepts anything.
    pub group: GroupPanel,
    pub attributes: Vec<Field>,
    pub token: String,
    pub assign_action: String,
    pub assign_picker: PickerSpec,
    pub tags: TagsPanel,
    pub type_action: String,
    pub type_picker: PickerSpec,
    pub auto_confirm_active: bool,
    pub auto_confirm_action: String,
    pub actions: Vec<ActionRow>,
}

pub struct GroupPanel {
    pub current: String,
    /// Built in Rust, never glued together in the template: the name goes
    /// into an RSQL filter and has to be quoted and percent-encoded there.
    pub current_href: String,
    pub set_action: String,
    pub clear_action: String,
    pub picker: PickerSpec,
}

pub struct LogRow {
    pub time: Cell,
    pub task: String,
    pub status: String,
    pub error: String,
}

pub struct IncidentPanel {
    pub number: String,
    pub recorded: Cell,
    pub node: String,
    pub sequence: String,
    pub logs: Vec<LogRow>,
}

pub struct DsTypePanel {
    pub mandatory: Vec<ChipRow>,
    pub optional: Vec<ChipRow>,
    pub add_action: String,
    pub picker: PickerSpec,
}

pub enum InputKind {
    Text,
    TextArea,
    Checkbox,
    Select,
    Picker,
}

pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

pub struct FormFieldView {
    pub help: String,
    /// The one-line answer from the manual, printed under the control. Empty
    /// when nothing is written, so the form simply omits the line.
    pub hint: String,
    pub label: &'static str,
    pub name: &'static str,
    pub value: String,
    pub required: bool,
    pub checked: bool,
    pub input: InputKind,
    pub options: Vec<SelectOption>,
    pub picker: Option<PickerSpec>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "form.html")]
pub struct FormPage {
    pub layout: Layout,
    pub title: String,
    pub action: String,
    pub next: Option<String>,
    pub error: Option<String>,
    /// Set when the form cannot be shown at all (unverified payload, or a
    /// resource with no form spec) — the reason replaces the fields.
    pub blocked: Option<&'static str>,
    pub fields: Vec<FormFieldView>,
}

pub struct ViewManage {
    pub delete_action: String,
    pub duplicate_action: String,
    pub move_action: String,
}

#[derive(Template)]
#[template(path = "view_form.html")]
pub struct ViewFormPage {
    pub layout: Layout,
    pub title: &'static str,
    pub action: String,
    pub resource: String,
    pub name: String,
    pub q: String,
    pub sort: String,
    pub columns: String,
    pub limit: usize,
    pub error: Option<String>,
    pub manage: Option<ViewManage>,
}

pub struct SearchHit {
    pub href: String,
    pub label: String,
}

pub struct SearchSection {
    pub title: String,
    pub rows: Vec<SearchHit>,
}

#[derive(Template)]
#[template(path = "search.html")]
pub struct SearchPage {
    pub layout: Layout,
    pub q: String,
    pub empty: bool,
    pub sections: Vec<SearchSection>,
}

pub struct ConfigRow {
    pub key: String,
    pub value: String,
    pub global: bool,
}

#[derive(Template)]
#[template(path = "settings.html")]
pub struct SettingsPage {
    pub layout: Layout,
    pub tenant: String,
    pub username: String,
    pub mgmt_url: String,
    pub state_api: String,
    pub console_build: &'static str,
    pub server_build: String,
    pub server_node: String,
    pub server_role: String,
    /// Empty unless the component is a debug build. A debug node behaves
    /// identically and runs several times slower, so "why is this sluggish"
    /// should be answerable from the page rather than by guessing.
    pub console_profile: String,
    pub server_profile: String,
    pub configs: Vec<ConfigRow>,
    pub extensions: ExtensionsPanel,
}

/// One extension point of the node the console is talking to.
pub struct ExtensionRow {
    pub label: String,
    pub points: String,
    pub env_var: String,
    pub configured: bool,
    /// What breaks when nothing is configured here — the node's own words,
    /// not the console's paraphrase.
    pub absence: String,
    pub module: String,
    pub version: String,
    pub path: String,
    pub sites: Vec<String>,
    pub capabilities: Vec<String>,
    pub deadline: String,
}

/// One parameter a module reads.
pub struct ExtensionParamRow {
    pub name: String,
    pub origin: String,
    /// Never the value of a secret: those are settable and never readable.
    pub value: Cell,
    /// The same thing as plain text, for an input's `value` attribute. A
    /// `Cell` renders markup, which cannot go inside an attribute. Empty for
    /// a secret, always.
    pub value_text: String,
    pub secret: bool,
    pub set: bool,
    pub editable: bool,
    pub removable: bool,
    pub changed: bool,
    /// Why this row has no form, when it has none.
    pub note: String,
}

/// The two node-scoped panels on `/settings`.
///
/// One struct rather than two so a node that will not answer produces one
/// coherent state rather than two panels disagreeing about whether it did.
pub struct ExtensionsPanel {
    pub available: bool,
    pub error: String,
    /// "node 1 (control)" — the console asks exactly one node, and the page
    /// must not let a reader assume it speaks for the fleet.
    pub node: String,
    pub writable: bool,
    pub rows: Vec<ExtensionRow>,
    pub params: Vec<ExtensionParamRow>,
    pub file_path: String,
    pub file_configured: bool,
    pub file_note: String,
    pub drift_note: String,
    /// What this node applies to every module, whatever a module declares.
    pub enforced_deadline: String,
}

impl ExtensionsPanel {
    /// The node did not answer, or refused to. The panels still render; the
    /// page frame is not allowed to depend on it (UI/UX rule 2).
    pub fn unavailable(reason: &str) -> Self {
        ExtensionsPanel {
            available: false,
            error: reason.to_string(),
            node: String::new(),
            writable: false,
            rows: Vec::new(),
            params: Vec::new(),
            file_path: String::new(),
            file_configured: false,
            file_note: String::new(),
            drift_note: String::new(),
            enforced_deadline: String::new(),
        }
    }
}

/// Turn the node's answer into the two panels.
pub fn extensions_panel(value: &Value) -> ExtensionsPanel {
    let node = &value["node"];
    let role = text_of(node, "role");
    // The node id is a number on the wire; text_of only speaks strings.
    let id = render_plain_value(&node["id"]);
    let writable = node["writable"].as_bool().unwrap_or(false);
    let file_path = text_of(node, "paramsFile");
    let file_state = text_of(node, "paramsFileState");
    let file_writable = node["paramsFileWritable"].as_bool().unwrap_or(false);
    let detail = text_of(node, "paramsFileDetail");

    // Every branch says what an operator can do about it. "Read-only" with
    // no reason is the state that makes people refresh the page.
    let file_note = if !writable {
        "This is a sub node: extension parameters are set on the control node.".to_string()
    } else if file_state == "unconfigured" {
        "This node has no parameters file. Set HAWIER_EXT_PARAMS_FILE to edit \
         parameters here; what the environment provides is listed below."
            .to_string()
    } else if file_state == "absent" {
        format!("{file_path} does not exist yet — saving a parameter creates it.")
    } else if file_state == "unreadable" {
        format!(
            "{file_path} cannot be read right now ({detail}). This node is \
             running with what it loaded at startup."
        )
    } else if !file_writable {
        format!("{file_path} is not writable by this node, so parameters are read-only here.")
    } else {
        String::new()
    };

    let drift = &value["drift"];
    let changed = drift["changed"].as_u64().unwrap_or(0);
    let drift_note = if !drift["readable"].as_bool().unwrap_or(true) {
        // Unreadable is not unchanged, and saying "0 changes" here would be a
        // lie an operator acts on.
        format!(
            "The parameters file cannot be read, so this node cannot say \
             whether it has changed ({}).",
            text_of(drift, "detail")
        )
    } else if changed == 0 {
        String::new()
    } else {
        format!(
            "{changed} parameter{} changed since node {id} ({role}) started. \
             Restart that node to apply; restarting may also change what its \
             modules are allowed to reach.",
            if changed == 1 { "" } else { "s" }
        )
    };

    let rows = value["points"]
        .as_array()
        .map(|points| points.iter().map(extension_row).collect())
        .unwrap_or_default();
    let params = value["parameters"]
        .as_array()
        .map(|params| {
            params
                .iter()
                .map(|param| extension_param_row(param, writable, file_writable))
                .collect()
        })
        .unwrap_or_default();

    ExtensionsPanel {
        available: true,
        error: String::new(),
        node: format!("node {id} ({role})"),
        writable,
        rows,
        params,
        file_path,
        file_configured: file_state != "unconfigured",
        file_note,
        drift_note,
        enforced_deadline: match node["enforcedDeadlineMs"].as_u64() {
            Some(ms) => format!("{ms} ms"),
            None => String::new(),
        },
    }
}

fn extension_row(point: &Value) -> ExtensionRow {
    let module = &point["module"];
    let list = |key: &str| -> Vec<String> {
        module[key]
            .as_array()
            .map(|items| items.iter().map(render_plain_value).collect())
            .unwrap_or_default()
    };
    let declared = module["declaredDeadlineMs"].as_u64();
    ExtensionRow {
        label: text_of(point, "label"),
        points: point["points"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(render_plain_value)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        env_var: text_of(point, "envVar"),
        configured: point["configured"].as_bool().unwrap_or(false),
        absence: text_of(point, "absence"),
        module: text_of(module, "name"),
        version: text_of(module, "version"),
        path: text_of(module, "path")
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string(),
        sites: list("sites"),
        capabilities: list("capabilities"),
        // Two facts, not one: a module declares a deadline and the host has
        // never used it.
        deadline: match declared {
            Some(ms) => format!("declares {ms} ms"),
            None => String::new(),
        },
    }
}

fn extension_param_row(
    param: &Value,
    node_writable: bool,
    file_writable: bool,
) -> ExtensionParamRow {
    let name = text_of(param, "name");
    let secret = param["secret"].as_bool().unwrap_or(false);
    let set = param["set"].as_bool().unwrap_or(false);
    let origin = text_of(param, "origin");
    let in_file = param["inFile"].as_bool().unwrap_or(false);
    let editable = param["editable"].as_bool().unwrap_or(false) && node_writable && file_writable;

    let note = if origin == "environment" && in_file {
        "Pinned by this node's environment; the line in the file is ignored.".to_string()
    } else if origin == "environment" {
        "Pinned by this node's environment.".to_string()
    } else {
        String::new()
    };

    let plain = text_of(param, "value");
    ExtensionParamRow {
        value: if secret {
            Cell::Muted(if set {
                "set".to_string()
            } else {
                "not set".to_string()
            })
        } else {
            Cell::text(plain.clone())
        },
        value_text: if secret { String::new() } else { plain },
        name,
        origin,
        secret,
        set,
        editable,
        removable: editable && in_file,
        changed: param["changedSinceStart"].as_bool().unwrap_or(false),
        note,
    }
}

pub struct GroupActionRow {
    pub href: String,
    pub controller_id: String,
    pub detail: String,
    pub kind: String,
    pub status: String,
    pub created: Cell,
}

#[derive(Template)]
#[template(path = "rollout_group.html")]
pub struct RolloutGroupPage {
    pub layout: Layout,
    pub name: String,
    pub rollout_id: String,
    pub rollout_href: String,
    pub status: String,
    pub total: i64,
    pub pager: Pager,
    pub rows: Vec<GroupActionRow>,
}

pub struct StatusRow {
    pub time: Cell,
    pub kind: String,
    pub code: String,
    pub messages: Vec<String>,
    /// How many times the device reported this exact state (1 = once). The
    /// server collapses verbatim repeats into one row, so without this the
    /// history would hide a device stuck in a retry loop.
    pub reports: i64,
    /// When the last of those reports arrived; only shown when `reports > 1`.
    pub last_time: Cell,
}

#[derive(Template)]
#[template(path = "action.html")]
pub struct ActionPage {
    pub layout: Layout,
    pub action_id: String,
    pub controller_id: String,
    pub target_href: String,
    pub cancellable: bool,
    pub cancel_action: String,
    pub detail: String,
    pub status: String,
    pub kind: String,
    pub distribution_set: Cell,
    pub statuses: Vec<StatusRow>,
}

// ---------------------------------------------------------------------------
// Fragments (returned to fetch() calls, not full pages)
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "partials/live_search.html")]
pub struct LiveSearchFragment {
    pub sections: Vec<SearchSection>,
}

pub struct RecentSearchRow {
    pub href: String,
    pub q: String,
    pub delete_action: String,
}

#[derive(Template)]
#[template(path = "partials/recent_searches.html")]
pub struct RecentSearchesFragment {
    pub recent: Vec<RecentSearchRow>,
}

pub struct PickRow {
    pub value: String,
    pub label: String,
}

#[derive(Template)]
#[template(path = "partials/pick.html")]
pub struct PickFragment {
    pub rows: Vec<PickRow>,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Projections used by the route handlers
// ---------------------------------------------------------------------------

/// Credentials the generic property table must not print. They have their
/// own panel with a deliberate reveal — a secret sitting in plain sight on
/// a page anyone can shoulder-surf defeats the point of masking it there.
const SECRET_FIELDS: [&str; 2] = ["securityToken", "gatewayToken"];

/// Object-page rows: every field except the HATEOAS plumbing.
pub fn fields_of(value: &Value, resource_id: &str) -> Vec<Field> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter(|(key, _)| key.as_str() != "_links")
        .map(|(key, field)| {
            if SECRET_FIELDS.contains(&key.as_str()) {
                let masked = "•••••••• (use the panel below to reveal it)".to_string();
                return Field {
                    help: help::field_key(resource_id, key),
                    key: key.clone(),
                    cell: Cell::Muted(masked.clone()),
                    raw: masked,
                };
            }
            Field {
                help: help::field_key(resource_id, key),
                key: key.clone(),
                cell: field_cell(key, Some(field), Some(value), true),
                raw: crate::html::raw_value(field),
            }
        })
        .collect()
}

/// A counter that failed to load shows as an em dash — never as a zero: a
/// wrong "0" on a dashboard is worse than an honest "no data".
pub fn count_text(count: Option<u64>) -> String {
    count.map_or_else(|| "—".to_string(), |value| value.to_string())
}

pub fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn str_or(value: &Value, key: &str, fallback: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub fn id_of(value: &Value, key: &str) -> String {
    value.get(key).map(render_plain_value).unwrap_or_default()
}

/// `field==*needle*` search link target for a status card.
pub fn status_card(status: &str, count: Option<u64>) -> StatCard {
    StatCard {
        href: format!(
            "/targets?q={}",
            encode_component(&format!("updatestatus=={status}"))
        ),
        label: status.to_string(),
        count: count_text(count),
    }
}

pub fn cell_at(row: &Value, path: &str) -> Cell {
    field_cell(path, read_path(row, path), Some(row), false)
}

// ---------------------------------------------------------------------------
// The manual
// ---------------------------------------------------------------------------

/// The manual is `'static`: pages borrow it rather than cloning prose.
pub struct HelpTopicLink {
    pub href: String,
    pub title: &'static str,
    pub summary: String,
}

pub struct HelpArea {
    pub label: &'static str,
    pub anchor: String,
    pub topics: Vec<HelpTopicLink>,
}

pub struct HelpHit {
    pub href: String,
    pub title: &'static str,
    pub topic_title: &'static str,
    pub summary: String,
}

#[derive(Template)]
#[template(path = "help_index.html")]
pub struct HelpIndexPage {
    pub layout: Layout,
    pub q: String,
    pub areas: Vec<HelpArea>,
    pub hits: Vec<HelpHit>,
}

pub struct HelpSectionView {
    /// Set on glossary entries: the heading is drawn as the badge itself.
    pub status: Option<help::Status>,
    pub anchor: &'static str,
    pub title: &'static str,
    pub body: &'static [Block],
}

#[derive(Template)]
#[template(path = "help_topic.html")]
pub struct HelpTopicPage {
    pub layout: Layout,
    pub topic_id: &'static str,
    pub title: &'static str,
    pub intro: &'static [Block],
    pub sections: Vec<HelpSectionView>,
}

/// The What's-This panel. Never an error page: a key nobody has written yet
/// answers with a plain sentence and a way into the manual.
#[derive(Template)]
#[template(path = "partials/help_panel.html")]
pub struct HelpPanelFragment {
    pub found: bool,
    pub topic_title: &'static str,
    pub title: &'static str,
    pub summary: &'static [Block],
    pub article_href: String,
}

/// Build the table of contents, chapter by chapter.
pub fn help_areas() -> Vec<HelpArea> {
    help::AREAS
        .iter()
        .map(|area| HelpArea {
            label: area.label(),
            anchor: help::slug(area.label()),
            topics: help::manual()
                .topics()
                .iter()
                .filter(|topic| topic.area == *area)
                .map(|topic| HelpTopicLink {
                    href: format!("/help/{}", topic.id),
                    title: &topic.title,
                    summary: topic.summary(),
                })
                .collect(),
        })
        .filter(|area| !area.topics.is_empty())
        .collect()
}
