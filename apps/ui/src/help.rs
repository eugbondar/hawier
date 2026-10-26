//! The operator manual.
//!
//! Prose lives in `apps/ui/docs/**.md` and is compiled into the binary with
//! `include_str!`. It is parsed once, on first use, into the typed blocks
//! below; the templates turn those into markup. Nothing here produces markup —
//! the manual obeys the same rule as every other page (CLAUDE.md, UI/UX 19).
//!
//! A control in the console points at a section by key, `topic#anchor`, and
//! carries that key in a `data-help` attribute. Anchors are derived from
//! headings by [`slug`], and the same function turns a column name or a status
//! word into the anchor that must document it — so "this column is explained"
//! is a mechanical check, not a table someone has to maintain.
//!
//! Writing convention: **the first paragraph of a section is the answer.**
//! That paragraph is what the What's-This panel shows and what a form field
//! prints as its hint; everything after it is the article.

use std::collections::HashMap;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Keys used when nothing more specific is written
// ---------------------------------------------------------------------------

/// A list contributed by an extension at runtime: the console cannot document
/// what it did not compile in, but it can say where the list came from.
pub const UNKNOWN_RESOURCE: &str = "console/surfaces#declared-by-an-extension";
pub const UNKNOWN_VIEW: &str = "console/views#system-views";
pub const UNKNOWN_FIELD: &str = "reference/columns#an-undocumented-column";
pub const UNKNOWN_STATUS: &str = "glossary#status-values";
pub const UNKNOWN_PAGE: &str = "console/help#the-manual";

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Chapters of the manual, in reading order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Start,
    Inventory,
    Repository,
    Deployment,
    Administration,
    Reference,
}

pub const AREAS: &[Area] = &[
    Area::Start,
    Area::Inventory,
    Area::Repository,
    Area::Deployment,
    Area::Administration,
    Area::Reference,
];

impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Area::Start => "Start here",
            Area::Inventory => "Inventory",
            Area::Repository => "Repository",
            Area::Deployment => "Deployment",
            Area::Administration => "Administration",
            Area::Reference => "Reference",
        }
    }
}

pub enum Span {
    Text(String),
    Code(String),
    Strong(String),
    /// A status word, drawn exactly as the lists draw it. The manual explains
    /// the console's colour language by speaking it.
    Status {
        href: String,
        word: &'static str,
        class: &'static str,
    },
    /// `doc_key` is set only for links that stay inside the manual, so the
    /// audit can check that they still land somewhere.
    Link {
        href: String,
        label: String,
        doc_key: Option<String>,
    },
}

pub struct Def {
    /// Spans, not text: a term is often the name of another section, and it
    /// must render as the link the writer typed rather than as its markup.
    pub term: Vec<Span>,
    pub text: Vec<Span>,
}

pub enum Block {
    Para(Vec<Span>),
    Bullets(Vec<Vec<Span>>),
    Steps(Vec<Vec<Span>>),
    Defs(Vec<Def>),
    Code(String),
    Note(Vec<Span>),
}

pub struct Section {
    /// Set on glossary entries only: there the heading *is* the badge, so the
    /// page reads as the legend for every list. Elsewhere a section called
    /// `error` is the name of a field, not a state.
    pub status: Option<Status>,
    pub anchor: String,
    /// `topic#anchor` — what a `data-help` attribute carries.
    pub key: String,
    pub title: String,
    /// Leading paragraphs: the short answer, shown in the panel.
    pub summary: Vec<Block>,
    pub body: Vec<Block>,
}

pub struct Topic {
    pub id: &'static str,
    pub area: Area,
    pub title: String,
    pub intro: Vec<Block>,
    pub sections: Vec<Section>,
}

impl Topic {
    /// The line under a topic in the table of contents.
    pub fn summary(&self) -> String {
        first_paragraph(&self.intro)
    }
}

impl Section {
    /// The line the What's-This panel leads with, and what a form field prints
    /// as its hint.
    pub fn summary_line(&self) -> String {
        first_paragraph(&self.summary)
    }
}

fn first_paragraph(blocks: &[Block]) -> String {
    for block in blocks {
        if let Block::Para(spans) = block {
            return plain(spans);
        }
    }
    // A section that opens with a list or a definition table still owes the
    // reader a line: a search hit or a field hint must never come out blank.
    blocks.first().map(plain_block).unwrap_or_default()
}

pub struct Manual {
    topics: Vec<Topic>,
    by_id: HashMap<&'static str, usize>,
    by_key: HashMap<String, (usize, usize)>,
}

impl Manual {
    pub fn topics(&self) -> &[Topic] {
        &self.topics
    }
}

// ---------------------------------------------------------------------------
// The registry. A topic is declared here, never discovered by walking a
// directory: a renamed file must be a compile error, not a silently missing
// chapter.
// ---------------------------------------------------------------------------

include!("help_topics.rs");

static MANUAL: OnceLock<Manual> = OnceLock::new();

pub fn manual() -> &'static Manual {
    MANUAL.get_or_init(|| {
        let topics: Vec<Topic> = TOPICS
            .iter()
            .map(|(id, area, source)| parse(id, *area, source))
            .collect();
        let mut by_id = HashMap::new();
        let mut by_key = HashMap::new();
        for (index, topic) in topics.iter().enumerate() {
            by_id.insert(topic.id, index);
            for (position, section) in topic.sections.iter().enumerate() {
                by_key.insert(section.key.clone(), (index, position));
            }
        }
        Manual {
            topics,
            by_id,
            by_key,
        }
    })
}

/// Anchors are always slugs, so a key written with a wire word — `in_sync`,
/// `wait_for_confirmation` — finds the section a heading produced. Lets a
/// template pass a status straight through without a Rust-side conversion.
pub fn normalise(key: &str) -> String {
    match key.split_once('#') {
        Some((id, anchor)) => format!("{id}#{}", slug(anchor)),
        None => key.to_string(),
    }
}

pub fn topic(id: &str) -> Option<&'static Topic> {
    let manual = manual();
    manual.by_id.get(id).map(|index| &manual.topics[*index])
}

/// Resolve `topic#anchor`. A key without an anchor resolves to the topic alone.
pub fn resolve(key: &str) -> Option<(&'static Topic, Option<&'static Section>)> {
    let manual = manual();
    let key = &normalise(key);
    if let Some((topic_index, section_index)) = manual.by_key.get(key) {
        return Some((
            &manual.topics[*topic_index],
            Some(&manual.topics[*topic_index].sections[*section_index]),
        ));
    }
    let id = key.split('#').next().unwrap_or(key);
    topic(id).map(|found| (found, None))
}

/// True when the key names a section that someone has actually written.
pub fn has(key: &str) -> bool {
    manual().by_key.contains_key(&normalise(key))
}

pub fn section(key: &str) -> Option<&'static Section> {
    let manual = manual();
    manual
        .by_key
        .get(&normalise(key))
        .map(|(topic_index, section_index)| &manual.topics[*topic_index].sections[*section_index])
}

/// The one-line answer behind a key — what a field prints under its control.
/// Empty when nothing is written, so a caller can leave the hint out entirely.
pub fn summary_text(key: &str) -> String {
    let blocks = match resolve(key) {
        Some((_, Some(found))) => &found.summary,
        Some((found, None)) => &found.intro,
        None => return String::new(),
    };
    for block in blocks {
        if let Block::Para(spans) = block {
            return plain(spans);
        }
    }
    String::new()
}

/// Where a column or a property row sends the reader: the resource's own field
/// list first, then the shared names every resource carries.
pub fn field_key(resource_id: &str, field: &str) -> String {
    let anchor = slug(field);
    let specific = format!("fields/{resource_id}#{anchor}");
    if has(&specific) {
        return specific;
    }
    let shared = format!("fields/common#{anchor}");
    if has(&shared) {
        return shared;
    }
    UNKNOWN_FIELD.to_string()
}

/// Which chapter documents a list. Matched by id, the way [`crate::view::icon`]
/// matches a resource to its shape: a list declared by an extension at runtime
/// is not here, and answers with the note that says so.
pub fn resource_topic(resource_id: &str) -> Option<&'static str> {
    Some(match resource_id {
        "targets" => "inventory/targets",
        "target-groups" => "inventory/target-groups",
        "target-tags" => "inventory/target-tags",
        "target-types" => "inventory/target-types",
        "software-modules" => "repository/software-modules",
        "software-module-types" => "repository/sm-types",
        "distribution-sets" => "repository/distribution-sets",
        "distribution-set-tags" => "repository/distribution-set-tags",
        "distribution-set-types" => "repository/ds-types",
        "rollouts" => "deployment/rollouts",
        "actions" => "deployment/actions",
        "target-filters" => "deployment/target-filters",
        "incidents" => "admin/incidents",
        "action-log" => "admin/action-log",
        "roles" => "admin/roles",
        "grants" => "admin/grants",
        "webhooks" => "admin/webhooks",
        "webhook-deliveries" => "admin/webhook-deliveries",
        "retention-rules" => "admin/retention-rules",
        _ => return None,
    })
}

/// The key a whole page answers with — the last resort behind every other
/// control on it.
pub fn page_key(resource_id: Option<&str>) -> String {
    match resource_id {
        Some(id) => match resource_topic(id) {
            Some(chapter) if topic(chapter).is_some() => chapter.to_string(),
            _ => UNKNOWN_RESOURCE.to_string(),
        },
        None => UNKNOWN_PAGE.to_string(),
    }
}

/// A view is documented as a section of its list's chapter.
pub fn view_key(resource_id: &str, view_id: &str) -> String {
    if let Some(id) = resource_topic(resource_id) {
        let key = format!("{id}#{}", slug(view_id));
        if has(&key) {
            return key;
        }
    }
    UNKNOWN_VIEW.to_string()
}

/// Bulk operation ids are unique across the console, so one chapter holds
/// them all.
pub fn bulk_key(action_id: &str) -> String {
    let key = format!("bulk#{}", slug(action_id));
    if has(&key) {
        key
    } else {
        UNKNOWN_FIELD.to_string()
    }
}

/// A field of a create/edit form is documented in that form's chapter.
pub fn form_field_key(form_topic: &str, field: &str) -> String {
    let key = format!("{form_topic}#{}", slug(field));
    if has(&key) {
        key
    } else {
        UNKNOWN_FIELD.to_string()
    }
}

/// Every status word in the console points at the glossary.
pub fn status_key(status: &str) -> String {
    let key = format!("glossary#{}", slug(status));
    if has(&key) {
        key
    } else {
        UNKNOWN_STATUS.to_string()
    }
}

pub fn article_href(key: &str) -> String {
    match key.split_once('#') {
        Some((id, anchor)) => format!("/help/{id}#{anchor}"),
        None => format!("/help/{key}"),
    }
}

/// Substring search over titles, section headings and body text.
pub fn search(needle: &str) -> Vec<(&'static Topic, &'static Section)> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for topic in manual().topics.iter() {
        for section in topic.sections.iter() {
            let haystack = format!(
                "{} {} {}",
                topic.title,
                section.title,
                section
                    .body
                    .iter()
                    .map(plain_block)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            if haystack.contains(&needle) {
                hits.push((topic, section));
            }
        }
    }
    hits
}

// ---------------------------------------------------------------------------
// Status words
// ---------------------------------------------------------------------------

/// One status word and the colour the console gives it.
#[derive(Clone, Copy)]
pub struct Status {
    pub word: &'static str,
    pub class: &'static str,
}

/// Every word the console prints as a state, and its colour. This is the only
/// such table: lists, detail panels and the manual all read it, so a status
/// cannot be green in one place and grey in another.
///
/// Green is healthy or progressing, amber is waiting, red is a failure, and
/// the rest are grey. Words that are *not* states — action types (`forced`,
/// `soft`) and action kinds (`update`, `cancel`) — are deliberately absent:
/// the console prints those plain, and so does the manual.
const STATUS: &[Status] = &[
    Status {
        word: "ok",
        class: "ok",
    },
    Status {
        word: "success",
        class: "ok",
    },
    Status {
        word: "finished",
        class: "ok",
    },
    Status {
        word: "running",
        class: "ok",
    },
    Status {
        word: "ready",
        class: "ok",
    },
    Status {
        word: "in_sync",
        class: "ok",
    },
    Status {
        word: "responding",
        class: "ok",
    },
    Status {
        word: "download",
        class: "ok",
    },
    Status {
        word: "downloaded",
        class: "ok",
    },
    Status {
        word: "active",
        class: "ok",
    },
    Status {
        word: "pending",
        class: "warn",
    },
    Status {
        word: "paused",
        class: "warn",
    },
    Status {
        word: "warning",
        class: "warn",
    },
    Status {
        word: "creating",
        class: "warn",
    },
    Status {
        word: "starting",
        class: "warn",
    },
    Status {
        word: "stopping",
        class: "warn",
    },
    Status {
        word: "deleting",
        class: "warn",
    },
    Status {
        word: "scheduled",
        class: "warn",
    },
    Status {
        word: "retrieved",
        class: "warn",
    },
    Status {
        word: "started",
        class: "warn",
    },
    Status {
        word: "wait_for_confirmation",
        class: "warn",
    },
    Status {
        word: "waiting_for_approval",
        class: "warn",
    },
    Status {
        word: "error",
        class: "error",
    },
    Status {
        word: "failed",
        class: "error",
    },
    Status {
        word: "cancelled",
        class: "error",
    },
    Status {
        word: "canceled",
        class: "error",
    },
    Status {
        word: "canceling",
        class: "error",
    },
    Status {
        word: "cancel_rejected",
        class: "error",
    },
    Status {
        word: "approval_denied",
        class: "error",
    },
    Status {
        word: "stopped",
        class: "error",
    },
    Status {
        word: "overdue",
        class: "error",
    },
    Status {
        word: "unknown",
        class: "neutral",
    },
    Status {
        word: "registered",
        class: "neutral",
    },
    Status {
        word: "deleted",
        class: "neutral",
    },
    Status {
        word: "inactive",
        class: "neutral",
    },
];

/// Every status word the console knows, for the audit that keeps this table
/// and the glossary in step.
pub fn statuses() -> &'static [Status] {
    STATUS
}

static STATUS_BY_SLUG: OnceLock<HashMap<String, Status>> = OnceLock::new();

/// The colour a status word carries, or `None` when the word is not a state.
/// Keyed by [`slug`], so `in_sync`, `in sync`, `IN-SYNC` and the enum's own
/// `InSync` all find the same row.
pub fn status(value: &str) -> Option<Status> {
    STATUS_BY_SLUG
        .get_or_init(|| {
            STATUS
                .iter()
                .map(|entry| (slug(entry.word), *entry))
                .collect()
        })
        .get(&slug(value))
        .copied()
}

// ---------------------------------------------------------------------------
// Plain text
// ---------------------------------------------------------------------------

fn plain(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Text(text) | Span::Code(text) | Span::Strong(text) => out.push_str(text),
            Span::Status { word, .. } => out.push_str(word),
            Span::Link { label, .. } => out.push_str(label),
        }
    }
    out
}

fn plain_block(block: &Block) -> String {
    match block {
        Block::Para(spans) | Block::Note(spans) => plain(spans),
        Block::Bullets(items) | Block::Steps(items) => {
            items.iter().map(|i| plain(i)).collect::<Vec<_>>().join(" ")
        }
        Block::Defs(defs) => defs
            .iter()
            .map(|def| format!("{} — {}", plain(&def.term), plain(&def.text)))
            .collect::<Vec<_>>()
            .join(" "),
        Block::Code(text) => text.clone(),
    }
}

// ---------------------------------------------------------------------------
// Anchors
// ---------------------------------------------------------------------------

/// `updateStatus` and `Update status` both become `update-status`, so a heading
/// documents a field without anyone writing the pair down twice.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut after_lower = false;
    for ch in text.chars() {
        if ch.is_ascii_uppercase() {
            if after_lower && !out.ends_with('-') {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            after_lower = false;
        } else if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            after_lower = true;
        } else {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            after_lower = false;
        }
    }
    out.trim_matches('-').to_string()
}

// ---------------------------------------------------------------------------
// The parser. One pass over the lines; the only state is which block is open.
// ---------------------------------------------------------------------------

enum Pending {
    None,
    Para(Vec<String>),
    Bullets(Vec<String>),
    Steps(Vec<String>),
    Defs(Vec<(String, String)>),
    Note(Vec<String>),
    Code(Vec<String>),
}

struct Builder {
    topic_id: &'static str,
    pending: Pending,
    blocks: Vec<Block>,
}

impl Builder {
    fn flush(&mut self) {
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        let id = self.topic_id;
        match pending {
            Pending::None => {}
            Pending::Para(lines) => self.blocks.push(Block::Para(spans(id, &lines.join(" ")))),
            Pending::Note(lines) => self.blocks.push(Block::Note(spans(id, &lines.join(" ")))),
            Pending::Bullets(items) => self.blocks.push(Block::Bullets(
                items.iter().map(|item| spans(id, item)).collect(),
            )),
            Pending::Steps(items) => self.blocks.push(Block::Steps(
                items.iter().map(|item| spans(id, item)).collect(),
            )),
            Pending::Defs(rows) => self.blocks.push(Block::Defs(
                rows.iter()
                    .map(|(term, text)| Def {
                        term: spans(id, term),
                        text: spans(id, text),
                    })
                    .collect(),
            )),
            Pending::Code(lines) => self.blocks.push(Block::Code(lines.join("\n"))),
        }
    }

    fn take(&mut self) -> Vec<Block> {
        self.flush();
        std::mem::take(&mut self.blocks)
    }
}

fn parse(id: &'static str, area: Area, source: &str) -> Topic {
    let mut title = String::new();
    let mut intro: Vec<Block> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut open: Option<(String, String)> = None;
    let mut builder = Builder {
        topic_id: id,
        pending: Pending::None,
        blocks: Vec::new(),
    };

    let mut close = |builder: &mut Builder, open: &mut Option<(String, String)>| {
        let blocks = builder.take();
        match open.take() {
            Some((anchor, heading)) => {
                let summary = leading_paragraphs(&blocks);
                sections.push(Section {
                    // Only the glossary turns its headings into badges.
                    status: if id == "glossary" {
                        status(&heading)
                    } else {
                        None
                    },
                    key: format!("{id}#{anchor}"),
                    anchor,
                    title: heading,
                    summary,
                    body: blocks,
                });
            }
            None => intro = blocks,
        }
    };

    for raw in source.lines() {
        if let Pending::Code(lines) = &mut builder.pending {
            if raw.trim_end() == "```" {
                builder.flush();
            } else {
                lines.push(raw.to_string());
            }
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            builder.flush();
            continue;
        }
        if line == "```" {
            builder.flush();
            builder.pending = Pending::Code(Vec::new());
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            close(&mut builder, &mut open);
            let (heading, anchor) = heading_anchor(rest);
            open = Some((anchor, heading));
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            if title.is_empty() {
                title = rest.trim().to_string();
                continue;
            }
        }
        if let Some(rest) = line.strip_prefix("- ") {
            match &mut builder.pending {
                Pending::Bullets(items) => items.push(rest.to_string()),
                _ => {
                    builder.flush();
                    builder.pending = Pending::Bullets(vec![rest.to_string()]);
                }
            }
            continue;
        }
        if let Some(rest) = strip_step(line) {
            match &mut builder.pending {
                Pending::Steps(items) => items.push(rest.to_string()),
                _ => {
                    builder.flush();
                    builder.pending = Pending::Steps(vec![rest.to_string()]);
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("> ") {
            match &mut builder.pending {
                Pending::Note(lines) => lines.push(rest.to_string()),
                _ => {
                    builder.flush();
                    builder.pending = Pending::Note(vec![rest.to_string()]);
                }
            }
            continue;
        }
        if let Some((term, text)) = line.split_once(" :: ") {
            let row = (term.trim().to_string(), text.trim().to_string());
            match &mut builder.pending {
                Pending::Defs(rows) => rows.push(row),
                _ => {
                    builder.flush();
                    builder.pending = Pending::Defs(vec![row]);
                }
            }
            continue;
        }
        match &mut builder.pending {
            Pending::Para(lines) => lines.push(line.to_string()),
            _ => {
                builder.flush();
                builder.pending = Pending::Para(vec![line.to_string()]);
            }
        }
    }
    close(&mut builder, &mut open);

    // `> one line` directly under the title is the topic's summary, not a
    // callout: it is what the table of contents and the panel lead with.
    if let Some(Block::Note(spans)) = intro.first() {
        let spans = clone_spans(spans);
        intro[0] = Block::Para(spans);
    }

    Topic {
        id,
        area,
        title,
        intro,
        sections,
    }
}

/// `Success threshold {#success-expression}` — an explicit anchor keeps a key
/// stable when the heading has to be reworded.
fn heading_anchor(rest: &str) -> (String, String) {
    let rest = rest.trim();
    if let Some(start) = rest.rfind("{#") {
        if rest.ends_with('}') {
            let anchor = rest[start + 2..rest.len() - 1].trim().to_string();
            return (rest[..start].trim().to_string(), anchor);
        }
    }
    (rest.to_string(), slug(rest))
}

fn strip_step(line: &str) -> Option<&str> {
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    line[digits.len()..].strip_prefix(". ")
}

fn leading_paragraphs(blocks: &[Block]) -> Vec<Block> {
    let mut summary = Vec::new();
    for block in blocks {
        match block {
            Block::Para(spans) => summary.push(Block::Para(clone_spans(spans))),
            _ if summary.is_empty() => {
                summary.push(clone_block(block));
                break;
            }
            _ => break,
        }
    }
    summary
}

fn clone_spans(spans: &[Span]) -> Vec<Span> {
    spans
        .iter()
        .map(|span| match span {
            Span::Text(text) => Span::Text(text.clone()),
            Span::Code(text) => Span::Code(text.clone()),
            Span::Strong(text) => Span::Strong(text.clone()),
            Span::Status { href, word, class } => Span::Status {
                href: href.clone(),
                word,
                class,
            },
            Span::Link {
                href,
                label,
                doc_key,
            } => Span::Link {
                href: href.clone(),
                label: label.clone(),
                doc_key: doc_key.clone(),
            },
        })
        .collect()
}

fn clone_block(block: &Block) -> Block {
    match block {
        Block::Para(spans) => Block::Para(clone_spans(spans)),
        Block::Note(spans) => Block::Note(clone_spans(spans)),
        Block::Bullets(items) => Block::Bullets(items.iter().map(|i| clone_spans(i)).collect()),
        Block::Steps(items) => Block::Steps(items.iter().map(|i| clone_spans(i)).collect()),
        Block::Defs(defs) => Block::Defs(
            defs.iter()
                .map(|def| Def {
                    term: clone_spans(&def.term),
                    text: clone_spans(&def.text),
                })
                .collect(),
        ),
        Block::Code(text) => Block::Code(text.clone()),
    }
}

/// Inline markup: `code`, **bold**, [label](target). An unterminated delimiter
/// stays literal text rather than eating the rest of the paragraph.
fn spans(topic_id: &str, text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '`' {
            if let Some(end) = find(&chars, index + 1, "`") {
                push_text(&mut out, &mut buffer);
                out.push(Span::Code(chars[index + 1..end].iter().collect()));
                index = end + 1;
                continue;
            }
        }
        if ch == '*' && chars.get(index + 1) == Some(&'*') {
            if let Some(end) = find(&chars, index + 2, "**") {
                push_text(&mut out, &mut buffer);
                out.push(Span::Strong(chars[index + 2..end].iter().collect()));
                index = end + 2;
                continue;
            }
        }
        if ch == '[' {
            if let Some(close) = find(&chars, index + 1, "]") {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(paren) = find(&chars, close + 2, ")") {
                        let label: String = chars[index + 1..close].iter().collect();
                        let target: String = chars[close + 2..paren].iter().collect();
                        push_text(&mut out, &mut buffer);
                        out.push(link(topic_id, &target, &label));
                        index = paren + 1;
                        continue;
                    }
                }
            }
        }
        buffer.push(ch);
        index += 1;
    }
    push_text(&mut out, &mut buffer);
    out
}

fn link(topic_id: &str, target: &str, label: &str) -> Span {
    let (href, doc_key) = if let Some(anchor) = target.strip_prefix('#') {
        (
            format!("/help/{topic_id}#{anchor}"),
            Some(format!("{topic_id}#{anchor}")),
        )
    } else if let Some(key) = target.strip_prefix("/help/") {
        (target.to_string(), Some(key.to_string()))
    } else {
        (target.to_string(), None)
    };
    // A link into the glossary draws the badge only when its label *is* the
    // word. That keeps prose reading as prose — "an error threshold pauses it"
    // stays a sentence, while "the device goes overdue" gets the red chip.
    if let Some(key) = doc_key.as_deref() {
        if let Some(anchor) = key.strip_prefix("glossary#") {
            if let Some(found) = status(anchor) {
                if found.word == label {
                    return Span::Status {
                        href,
                        word: found.word,
                        class: found.class,
                    };
                }
            }
        }
    }
    Span::Link {
        href,
        label: label.to_string(),
        doc_key,
    }
}

fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    let mut index = from;
    while index + needle.len() <= chars.len() {
        if chars[index..index + needle.len()] == needle[..] {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn push_text(out: &mut Vec<Span>, buffer: &mut String) {
    if !buffer.is_empty() {
        out.push(Span::Text(std::mem::take(buffer)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_match_field_names_and_headings() {
        assert_eq!(slug("updateStatus"), "update-status");
        assert_eq!(slug("Update status"), "update-status");
        assert_eq!(
            slug("lastControllerRequestAt"),
            "last-controller-request-at"
        );
        assert_eq!(slug("Error threshold % (pause)"), "error-threshold-pause");
        assert_eq!(slug("in_sync"), "in-sync");
        assert_eq!(slug("sha1"), "sha1");
        assert_eq!(slug("attribute.*"), "attribute");
    }

    #[test]
    fn the_line_under_the_title_is_the_topic_summary() {
        let topic = parse(
            "t",
            Area::Reference,
            "# Title\n\n> What it is.\n\n## One\n\n> careful\n",
        );
        assert_eq!(topic.summary(), "What it is.");
        assert!(matches!(topic.sections[0].body[0], Block::Note(_)));
    }

    #[test]
    fn a_heading_may_pin_its_anchor() {
        let (title, anchor) = heading_anchor("Success threshold {#success-expression}");
        assert_eq!(title, "Success threshold");
        assert_eq!(anchor, "success-expression");
    }

    #[test]
    fn every_block_kind_survives_a_round_trip() {
        let topic = parse(
            "t",
            Area::Reference,
            "# Title\n\n> Intro line.\n\n## First {#one}\n\nThe answer.\n\nMore about it.\n\n- a\n- b\n\n1. step one\n2. step two\n\nterm :: what it means\n\n> careful\n\n```\nname==prod-*\n```\n",
        );
        assert_eq!(topic.title, "Title");
        assert_eq!(topic.sections.len(), 1);
        let section = &topic.sections[0];
        assert_eq!(section.key, "t#one");
        assert_eq!(section.title, "First");
        // The answer is the first paragraph, and only that.
        assert_eq!(section.summary_line(), "The answer.");
        assert_eq!(section.body.len(), 7);
        assert!(matches!(section.body[2], Block::Bullets(ref rows) if rows.len() == 2));
        assert!(matches!(section.body[3], Block::Steps(ref rows) if rows.len() == 2));
        assert!(matches!(section.body[4], Block::Defs(ref rows) if plain(&rows[0].term) == "term"));
        assert!(matches!(section.body[5], Block::Note(_)));
        assert!(matches!(section.body[6], Block::Code(ref text) if text == "name==prod-*"));
    }

    #[test]
    fn inline_markup_and_links() {
        let spans = spans(
            "rollouts",
            "see `q` and **this** and [that](#groups) and [out](/targets)",
        );
        let doc: Vec<&Span> = spans
            .iter()
            .filter(|span| matches!(span, Span::Link { .. }))
            .collect();
        assert_eq!(doc.len(), 2);
        match doc[0] {
            Span::Link { href, doc_key, .. } => {
                assert_eq!(href, "/help/rollouts#groups");
                assert_eq!(doc_key.as_deref(), Some("rollouts#groups"));
            }
            _ => unreachable!(),
        }
        match doc[1] {
            Span::Link { href, doc_key, .. } => {
                assert_eq!(href, "/targets");
                assert!(doc_key.is_none());
            }
            _ => unreachable!(),
        }
        assert!(spans.iter().any(|s| matches!(s, Span::Code(t) if t == "q")));
        assert!(spans
            .iter()
            .any(|s| matches!(s, Span::Strong(t) if t == "this")));
    }

    #[test]
    fn an_unterminated_delimiter_stays_literal() {
        let spans = spans("t", "a ` b ** c [d](e");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0], Span::Text(ref t) if t == "a ` b ** c [d](e"));
    }

    #[test]
    fn a_wire_word_finds_the_section_a_heading_made() {
        assert_eq!(normalise("glossary#in_sync"), "glossary#in-sync");
        assert!(has("glossary#in_sync"));
        assert!(has("glossary#wait_for_confirmation"));
    }

    #[test]
    fn the_fallback_keys_are_written() {
        for key in [
            UNKNOWN_RESOURCE,
            UNKNOWN_VIEW,
            UNKNOWN_FIELD,
            UNKNOWN_STATUS,
            UNKNOWN_PAGE,
        ] {
            assert!(
                has(key),
                "{key} is a fallback with nothing written behind it"
            );
        }
    }

    #[test]
    fn no_topic_declares_the_same_anchor_twice() {
        for topic in manual().topics() {
            let mut seen = std::collections::HashSet::new();
            for section in topic.sections.iter() {
                assert!(
                    seen.insert(section.anchor.clone()),
                    "{}: two sections slug to '{}'",
                    topic.id,
                    section.anchor
                );
            }
            assert!(!topic.title.is_empty(), "{} has no title", topic.id);
        }
    }
}
