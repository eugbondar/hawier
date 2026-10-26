use std::collections::HashMap;

use serde_json::{json, Map, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Inventory,
    Deployment,
    Repository,
    Admin,
}

/// A resource as the console works with it.
///
/// Owned rather than `&'static`, because the registry is no longer only what
/// is compiled in: an extension declares its own tree node and views, and
/// they arrive at runtime (EXTENSIONS.md §4.5). The built-in table below
/// keeps its literal shape and is converted into these.
#[derive(Clone)]
pub struct Resource {
    pub id: String,
    pub label: String,
    pub endpoint: String,
    pub group: Group,
    pub id_field: String,
    pub columns: Vec<String>,
    pub default_sort: Option<String>,
    /// Text fields the global search matches with RSQL wildcards. Must be
    /// `Kind::Text` columns in the server's field map — anything else makes
    /// the query 400 (see CLAUDE.md, UI/UX rules).
    pub search_fields: Vec<String>,
    pub form: Option<FormSpec>,
    pub create_verified: bool,
    /// A row that opens a filtered list of *another* resource instead of its
    /// own object page. Target groups have no page of their own: the useful
    /// destination is the devices in the group, and that is also the query
    /// whose count the row shows (UI/UX rule 6).
    pub row_href: Option<RowHref>,
    /// Icon name. Built-ins are looked up by id; a declared surface has to
    /// bring its own, because in a collapsed sidebar the icon is the only
    /// thing telling two resources apart (UI/UX rule 14).
    pub icon: Option<String>,
}

/// The compiled-in form of [`Resource`]. Exists so the table below stays a
/// plain list of literals instead of a thousand `to_string()` calls.
#[derive(Clone, Copy)]
pub struct Builtin {
    pub id: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub group: Group,
    pub id_field: &'static str,
    pub columns: &'static [&'static str],
    pub default_sort: Option<&'static str>,
    pub search_fields: &'static [&'static str],
    pub form: Option<FormSpec>,
    pub create_verified: bool,
    pub row_href: Option<RowHref>,
}

impl From<&Builtin> for Resource {
    fn from(builtin: &Builtin) -> Self {
        Resource {
            id: builtin.id.to_string(),
            label: builtin.label.to_string(),
            endpoint: builtin.endpoint.to_string(),
            group: builtin.group,
            id_field: builtin.id_field.to_string(),
            columns: builtin.columns.iter().map(|c| c.to_string()).collect(),
            default_sort: builtin.default_sort.map(str::to_string),
            search_fields: builtin
                .search_fields
                .iter()
                .map(|f| f.to_string())
                .collect(),
            form: builtin.form,
            create_verified: builtin.create_verified,
            row_href: builtin.row_href,
            icon: None,
        }
    }
}

/// `/{resource}?q={field}=="{value}"` built from a column of the row.
#[derive(Clone, Copy)]
pub struct RowHref {
    pub resource: &'static str,
    pub field: &'static str,
    pub column: &'static str,
}

/// A view as the console works with it — owned, for the same reason as
/// [`Resource`].
#[derive(Clone)]
pub struct SystemView {
    pub id: String,
    pub label: String,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub columns: Option<Vec<String>>,
    /// Came from an extension rather than the table below. A built-in
    /// resource is recognisable by carrying no icon; a view had no such
    /// mark, so nothing — the audit included — could tell whether a
    /// declaration had reached the tree or been dropped on the floor.
    pub declared: bool,
}

/// The compiled-in form of [`SystemView`].
#[derive(Clone, Copy)]
pub struct BuiltinView {
    pub id: &'static str,
    pub label: &'static str,
    pub q: Option<&'static str>,
    pub sort: Option<&'static str>,
    pub columns: Option<&'static [&'static str]>,
}

impl From<&BuiltinView> for SystemView {
    fn from(view: &BuiltinView) -> Self {
        SystemView {
            id: view.id.to_string(),
            label: view.label.to_string(),
            q: view.q.map(str::to_string),
            sort: view.sort.map(str::to_string),
            columns: view
                .columns
                .map(|cols| cols.iter().map(|c| c.to_string()).collect()),
            declared: false,
        }
    }
}

#[derive(Clone, Copy)]
pub struct FormSpec {
    pub title: &'static str,
    pub kind: FormKind,
    pub fields: &'static [FormField],
}

#[derive(Clone, Copy)]
pub enum FormKind {
    Rollout,
    Target,
    TargetFilter,
    DistributionSet,
    SoftwareModule,
    /// name + optional description/colour (target/DS tags, target types).
    Tag,
    /// Tag fields + required `key` (DS types, SM types).
    TypeWithKey,
    /// Event subscription: where to POST, which kinds, signing secret.
    Webhook,
    /// Automatic deletion rule: schedule, what it may consider, dry run.
    RetentionRule,
    /// A named set of permissions.
    Role,
}

impl FormKind {
    /// Which chapter of the manual documents this form. Field help is that
    /// chapter plus the field name, so a new field needs a section, not a
    /// second table.
    pub fn help_topic(self) -> &'static str {
        match self {
            FormKind::Rollout => "forms/rollout",
            FormKind::Target => "forms/target",
            FormKind::TargetFilter => "forms/target-filter",
            FormKind::DistributionSet => "forms/distribution-set",
            FormKind::SoftwareModule => "forms/software-module",
            FormKind::Tag => "forms/tag",
            FormKind::TypeWithKey => "forms/type",
            FormKind::Webhook => "forms/webhook",
            FormKind::RetentionRule => "forms/retention-rule",
            FormKind::Role => "forms/role",
        }
    }
}

#[derive(Clone, Copy)]
pub enum PayloadMode {
    Create,
    Update,
}

#[derive(Clone, Copy)]
pub struct FormField {
    pub name: &'static str,
    pub label: &'static str,
    pub required: bool,
    pub input: FieldInput,
}

#[derive(Clone, Copy)]
pub enum FieldInput {
    Text,
    TextArea,
    Checkbox,
    /// Dropdown; options are static or fetched by the route handler.
    Select(SelectSource),
}

#[derive(Clone, Copy)]
pub enum SelectSource {
    Static(&'static [(&'static str, &'static str)]),
    /// Populated from GET /rest/v1/distributionsets by the form handler.
    DistributionSets,
}

#[derive(Debug)]
pub struct FormValidationError {
    pub field_errors: HashMap<String, String>,
}

impl FormValidationError {
    pub fn summary(&self) -> String {
        self.field_errors
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| "form validation failed".to_string())
    }
}

const TARGET_FIELDS: &[FormField] = &[
    FormField {
        name: "controllerId",
        label: "Controller ID",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "description",
        label: "Description",
        required: false,
        input: FieldInput::TextArea,
    },
    FormField {
        name: "securityToken",
        label: "Security token",
        required: false,
        input: FieldInput::Text,
    },
    FormField {
        name: "address",
        label: "Address",
        required: false,
        input: FieldInput::Text,
    },
];

const TARGET_FILTER_FIELDS: &[FormField] = &[
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "query",
        label: "FIQL query",
        required: true,
        input: FieldInput::TextArea,
    },
];

const DISTRIBUTION_SET_FIELDS: &[FormField] = &[
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "version",
        label: "Version",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "type",
        label: "Type",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "description",
        label: "Description",
        required: false,
        input: FieldInput::TextArea,
    },
    FormField {
        name: "requiredMigrationStep",
        label: "Requires migration step",
        required: false,
        input: FieldInput::Checkbox,
    },
];

const ROLLOUT_FIELDS: &[FormField] = &[
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "distributionSetId",
        label: "Distribution set",
        required: true,
        input: FieldInput::Select(SelectSource::DistributionSets),
    },
    FormField {
        name: "targetFilterQuery",
        label: "Target filter (RSQL)",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "amountGroups",
        label: "Groups",
        required: false,
        input: FieldInput::Text,
    },
    FormField {
        name: "successExpression",
        label: "Success threshold %",
        required: false,
        input: FieldInput::Text,
    },
    FormField {
        name: "errorExpression",
        label: "Error threshold % (pause)",
        required: false,
        input: FieldInput::Text,
    },
    FormField {
        name: "type",
        label: "Type",
        required: false,
        input: FieldInput::Select(SelectSource::Static(&[
            ("forced", "forced"),
            ("soft", "soft"),
            ("timeforced", "timeforced"),
            ("downloadonly", "download only"),
        ])),
    },
    FormField {
        name: "description",
        label: "Description",
        required: false,
        input: FieldInput::TextArea,
    },
];

const SOFTWARE_MODULE_FIELDS: &[FormField] = &[
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "version",
        label: "Version",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "type",
        label: "Type",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "vendor",
        label: "Vendor",
        required: false,
        input: FieldInput::Text,
    },
    FormField {
        name: "description",
        label: "Description",
        required: false,
        input: FieldInput::TextArea,
    },
];

const TAG_FIELDS: &[FormField] = &[
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "description",
        label: "Description",
        required: false,
        input: FieldInput::TextArea,
    },
    FormField {
        name: "colour",
        label: "Colour",
        required: false,
        input: FieldInput::Text,
    },
];

const TYPE_WITH_KEY_FIELDS: &[FormField] = &[
    FormField {
        name: "name",
        label: "Name",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "key",
        label: "Key",
        required: true,
        input: FieldInput::Text,
    },
    FormField {
        name: "description",
        label: "Description",
        required: false,
        input: FieldInput::TextArea,
    },
    FormField {
        name: "colour",
        label: "Colour",
        required: false,
        input: FieldInput::Text,
    },
    FormField {
        name: "maxAssignments",
        label: "Max assignments (SM types)",
        required: false,
        input: FieldInput::Text,
    },
];

const BUILTIN: &[Builtin] = &[
    Builtin {
        id: "targets",
        label: "Targets",
        endpoint: "targets",
        group: Group::Inventory,
        id_field: "controllerId",
        columns: &[
            "controllerId",
            "name",
            "group",
            "updateStatus",
            "assignedDistributionSetName",
            "pollState",
            "lastControllerRequestAt",
        ],
        default_sort: None,
        search_fields: &["controllerId", "name", "description"],
        form: Some(FormSpec {
            title: "New Target",
            kind: FormKind::Target,
            fields: TARGET_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "rollouts",
        label: "Rollouts",
        endpoint: "rollouts",
        group: Group::Deployment,
        id_field: "id",
        columns: &[
            "id",
            "name",
            "status",
            "totalTargets",
            "distributionSet",
            "createdAt",
        ],
        default_sort: Some("createdAt:DESC"),
        search_fields: &["name", "description"],
        form: Some(FormSpec {
            title: "New Rollout",
            kind: FormKind::Rollout,
            fields: ROLLOUT_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "distribution-sets",
        label: "Distribution Sets",
        endpoint: "distributionsets",
        group: Group::Deployment,
        id_field: "id",
        columns: &["id", "name", "version", "type", "complete", "createdAt"],
        default_sort: Some("createdAt:DESC"),
        search_fields: &["name", "version", "description"],
        form: Some(FormSpec {
            title: "New Distribution Set",
            kind: FormKind::DistributionSet,
            fields: DISTRIBUTION_SET_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "target-filters",
        label: "Target Filters",
        endpoint: "targetfilters",
        group: Group::Deployment,
        id_field: "id",
        columns: &[
            "id",
            "name",
            "query",
            "autoAssignDistributionSet",
            "createdAt",
        ],
        default_sort: Some("createdAt:DESC"),
        search_fields: &["name", "query"],
        form: Some(FormSpec {
            title: "New Target Filter",
            kind: FormKind::TargetFilter,
            fields: TARGET_FILTER_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "software-modules",
        label: "Software Modules",
        endpoint: "softwaremodules",
        group: Group::Repository,
        id_field: "id",
        columns: &["id", "name", "version", "type", "vendor", "createdAt"],
        default_sort: Some("createdAt:DESC"),
        search_fields: &["name", "version", "vendor", "description"],
        form: Some(FormSpec {
            title: "New Software Module",
            kind: FormKind::SoftwareModule,
            fields: SOFTWARE_MODULE_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        // Device update actions across the fleet — the landing page for
        // "what did this rollout group do to this device".
        id: "actions",
        label: "Actions",
        endpoint: "actions",
        group: Group::Deployment,
        id_field: "id",
        columns: &[
            "id",
            "controllerId",
            "detailStatus",
            "type",
            "rolloutName",
            "createdAt",
        ],
        default_sort: Some("id:DESC"),
        search_fields: &["target.controllerid"],
        form: None,
        create_verified: false,
        row_href: None,
    },
    Builtin {
        // hawier extensions on the mgmt surface (API_NOTES §5): plain
        // resources, so views/sorting/paging/saved views come for free.
        id: "incidents",
        label: "Incidents",
        endpoint: "incidents",
        group: Group::Admin,
        id_field: "incident",
        columns: &[
            "incident",
            "kind",
            "occurredAt",
            "status",
            "method",
            "path",
            "errorCode",
            "message",
        ],
        default_sort: Some("incident:DESC"),
        search_fields: &["path", "message", "errorCode"],
        form: None,
        create_verified: false,
        row_href: None,
    },
    Builtin {
        id: "roles",
        label: "Roles",
        endpoint: "roles",
        group: Group::Admin,
        id_field: "id",
        columns: &["name", "description", "permissions", "holders", "createdAt"],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "description"],
        form: Some(FormSpec {
            title: "New role",
            kind: FormKind::Role,
            fields: &[
                FormField {
                    name: "name",
                    label: "Name",
                    required: true,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "permissions",
                    label: "Permissions",
                    required: true,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "description",
                    label: "Description",
                    required: false,
                    input: FieldInput::Text,
                },
            ],
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        // Who holds what. A separate list because the question an operator
        // asks is "what can this person do", not "who has this role".
        id: "grants",
        label: "Grants",
        endpoint: "grants",
        group: Group::Admin,
        id_field: "id",
        columns: &["username", "roleName", "createdAt"],
        default_sort: Some("username:ASC"),
        search_fields: &["username"],
        form: None,
        create_verified: false,
        row_href: None,
    },
    Builtin {
        id: "retention-rules",
        label: "Retention Rules",
        endpoint: "retentionrules",
        group: Group::Admin,
        id_field: "id",
        columns: &[
            "name",
            "scope",
            "cron",
            "filter",
            "dryRun",
            "active",
            "lastRunAt",
            "lastResult",
        ],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "filter"],
        form: Some(FormSpec {
            title: "New retention rule",
            kind: FormKind::RetentionRule,
            fields: &[
                FormField {
                    name: "name",
                    label: "Name",
                    required: true,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "cron",
                    label: "Schedule (cron)",
                    required: true,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "filter",
                    label: "Filter (RSQL)",
                    required: false,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "minAgeMs",
                    label: "Minimum age (ms)",
                    required: false,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "dryRun",
                    label: "Dry run",
                    required: false,
                    input: FieldInput::Checkbox,
                },
            ],
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "webhooks",
        label: "Webhooks",
        endpoint: "webhooks",
        group: Group::Admin,
        id_field: "id",
        columns: &["name", "url", "eventMask", "active", "signed", "createdAt"],
        default_sort: Some("createdAt:DESC"),
        search_fields: &["name", "url"],
        form: Some(FormSpec {
            title: "New webhook",
            kind: FormKind::Webhook,
            fields: &[
                FormField {
                    name: "name",
                    label: "Name",
                    required: true,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "url",
                    label: "URL",
                    required: true,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "eventMask",
                    label: "Event mask",
                    required: false,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "secret",
                    label: "Signing secret",
                    required: false,
                    input: FieldInput::Text,
                },
                FormField {
                    name: "active",
                    label: "Active",
                    required: false,
                    input: FieldInput::Checkbox,
                },
            ],
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        // What actually went out. A subscription that quietly stopped working
        // is only visible here, so it is a resource rather than a panel.
        id: "webhook-deliveries",
        label: "Webhook Deliveries",
        endpoint: "webhookdeliveries",
        group: Group::Admin,
        id_field: "id",
        columns: &["deliveredAt", "webhookName", "kind", "status", "error"],
        default_sort: Some("deliveredAt:DESC"),
        search_fields: &["kind", "error"],
        form: None,
        create_verified: false,
        row_href: None,
    },
    Builtin {
        id: "action-log",
        label: "Action Log",
        endpoint: "actionlog",
        group: Group::Admin,
        id_field: "id",
        columns: &[
            "createdAt",
            "username",
            "resource",
            "resourceId",
            "action",
            "status",
            "error",
        ],
        default_sort: Some("createdAt:DESC"),
        search_fields: &["action", "resource", "error"],
        form: None,
        create_verified: false,
        row_href: None,
    },
    Builtin {
        // hawkBit's own /targetgroups answers with a bare array of strings —
        // no total, no filter, no sort — so the console reads the ordinary
        // resource hawier adds beside it (API_NOTES §5).
        id: "target-groups",
        label: "Target Groups",
        endpoint: "targetgroupdetails",
        group: Group::Inventory,
        id_field: "name",
        columns: &["name", "targets"],
        default_sort: Some("name:ASC"),
        search_fields: &["name"],
        form: None,
        create_verified: false,
        row_href: Some(RowHref {
            resource: "targets",
            field: "group",
            column: "name",
        }),
    },
    Builtin {
        id: "target-tags",
        label: "Target Tags",
        endpoint: "targettags",
        group: Group::Admin,
        id_field: "id",
        columns: &["id", "name", "description", "colour"],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "description"],
        form: Some(FormSpec {
            title: "New Target Tag",
            kind: FormKind::Tag,
            fields: TAG_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "target-types",
        label: "Target Types",
        endpoint: "targettypes",
        group: Group::Admin,
        id_field: "id",
        columns: &["id", "name", "key", "description", "colour"],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "key", "description"],
        form: Some(FormSpec {
            title: "New Target Type",
            kind: FormKind::TypeWithKey,
            fields: TYPE_WITH_KEY_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "distribution-set-tags",
        label: "DS Tags",
        endpoint: "distributionsettags",
        group: Group::Admin,
        id_field: "id",
        columns: &["id", "name", "description", "colour"],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "description"],
        form: Some(FormSpec {
            title: "New Distribution Set Tag",
            kind: FormKind::Tag,
            fields: TAG_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "distribution-set-types",
        label: "DS Types",
        endpoint: "distributionsettypes",
        group: Group::Admin,
        id_field: "id",
        columns: &["id", "name", "key", "description"],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "key", "description"],
        form: Some(FormSpec {
            title: "New Distribution Set Type",
            kind: FormKind::TypeWithKey,
            fields: TYPE_WITH_KEY_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
    Builtin {
        id: "software-module-types",
        label: "SM Types",
        endpoint: "softwaremoduletypes",
        group: Group::Admin,
        id_field: "id",
        columns: &["id", "name", "key", "maxAssignments", "description"],
        default_sort: Some("name:ASC"),
        search_fields: &["name", "key", "description"],
        form: Some(FormSpec {
            title: "New Software Module Type",
            kind: FormKind::TypeWithKey,
            fields: TYPE_WITH_KEY_FIELDS,
        }),
        create_verified: true,
        row_href: None,
    },
];

/// One bulk operation offered over a selection: the form value, the label,
/// and whether it needs an extra pick (a DS, a tag) from the toolbar.
#[derive(Clone, Copy)]
pub struct BulkAction {
    pub id: &'static str,
    pub label: &'static str,
    pub danger: bool,
    /// Name of the companion select rendered next to the button.
    pub picker: Option<BulkPicker>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BulkPicker {
    DistributionSet,
    TargetTag,
    DsTag,
    /// Free text with suggestions: a group path may not exist yet.
    TargetGroup,
}

const DELETE: BulkAction = BulkAction {
    id: "delete",
    label: "Delete",
    danger: true,
    picker: None,
};

/// What can be done to a selection, per resource.
pub fn bulk_actions(resource_id: &str) -> &'static [BulkAction] {
    match resource_id {
        // A delivery is a record of what happened; deleting one would only
        // hide it. Webhooks fall through to the default DELETE below.
        "webhook-deliveries" => &[],
        "targets" => &[
            BulkAction {
                id: "assign-ds",
                label: "Assign DS",
                danger: false,
                picker: Some(BulkPicker::DistributionSet),
            },
            BulkAction {
                id: "tag",
                label: "Add tag",
                danger: false,
                picker: Some(BulkPicker::TargetTag),
            },
            BulkAction {
                id: "untag",
                label: "Remove tag",
                danger: false,
                picker: Some(BulkPicker::TargetTag),
            },
            BulkAction {
                id: "set-group",
                label: "Set group",
                danger: false,
                picker: Some(BulkPicker::TargetGroup),
            },
            BulkAction {
                id: "clear-group",
                label: "Clear group",
                danger: false,
                picker: None,
            },
            BulkAction {
                id: "cancel-actions",
                label: "Cancel actions",
                danger: true,
                picker: None,
            },
            DELETE,
        ],
        "distribution-sets" => &[
            BulkAction {
                id: "tag",
                label: "Add tag",
                danger: false,
                picker: Some(BulkPicker::DsTag),
            },
            BulkAction {
                id: "untag",
                label: "Remove tag",
                danger: false,
                picker: Some(BulkPicker::DsTag),
            },
            BulkAction {
                id: "invalidate",
                label: "Invalidate",
                danger: true,
                picker: None,
            },
            DELETE,
        ],
        "rollouts" => &[
            BulkAction {
                id: "start",
                label: "Start",
                danger: false,
                picker: None,
            },
            BulkAction {
                id: "pause",
                label: "Pause",
                danger: false,
                picker: None,
            },
            BulkAction {
                id: "resume",
                label: "Resume",
                danger: false,
                picker: None,
            },
            BulkAction {
                id: "stop",
                label: "Stop",
                danger: true,
                picker: None,
            },
            DELETE,
        ],
        // Read-only journals and the derived group list have nothing to
        // act on (the fallback below would offer a Delete that goes nowhere).
        "incidents" | "action-log" | "actions" | "target-groups" => &[],
        _ => &[DELETE],
    }
}

/// Everything the console knows about: what is compiled in, plus whatever
/// extensions declared at startup.
///
/// A process-wide registry rather than state threaded through every handler:
/// it is set once at boot and read from templates, routes and the audit
/// alike. Before it is set — tests that never call [`install`] — only the
/// built-in table is visible, which is exactly the old behaviour.
static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();

#[derive(Default)]
struct Registry {
    resources: Vec<Resource>,
    /// Views contributed per resource id, appended after the built-in ones.
    views: std::collections::HashMap<String, Vec<SystemView>>,
}

/// Publish the extension-declared surfaces. Called once, at startup.
pub fn install(resources: Vec<Resource>, views: Vec<(String, Vec<SystemView>)>) {
    let mut map: std::collections::HashMap<String, Vec<SystemView>> =
        std::collections::HashMap::new();
    for (resource_id, declared) in views {
        map.entry(resource_id).or_default().extend(declared);
    }
    // Losing the race means another caller already installed the same thing;
    // there is nothing to recover from.
    let _ = REGISTRY.set(Registry {
        resources,
        views: map,
    });
}

/// Every resource, built-in first so a declaration cannot displace one.
pub fn all() -> Vec<Resource> {
    let mut resources: Vec<Resource> = BUILTIN.iter().map(Resource::from).collect();
    if let Some(registry) = REGISTRY.get() {
        for declared in &registry.resources {
            if resources.iter().any(|r| r.id == declared.id) {
                continue;
            }
            resources.push(declared.clone());
        }
    }
    resources
}

pub fn by_id(id: &str) -> Option<Resource> {
    all().into_iter().find(|resource| resource.id == id)
}

/// The views of a resource: the compiled-in ones, then anything declared.
pub fn system_views(resource_id: &str) -> Vec<SystemView> {
    let mut views: Vec<SystemView> = builtin_views(resource_id)
        .iter()
        .map(SystemView::from)
        .collect();
    if let Some(registry) = REGISTRY.get() {
        if let Some(declared) = registry.views.get(resource_id) {
            for view in declared {
                // A built-in of the same id wins — the console must keep
                // working whatever a module declares. But it used to win in
                // silence, and the single declared view this project shipped
                // was landing here: `rollouts/needs-attention` collided with
                // the built-in of that name and simply never rendered. Say so
                // rather than swallow it; it cannot be an error, because the
                // console has to start regardless.
                if views.iter().any(|v| v.id == view.id) {
                    tracing::warn!(
                        resource = resource_id,
                        view = view.id,
                        "declared view ignored: this resource already has a \
                         built-in view with that id"
                    );
                    continue;
                }
                views.push(view.clone());
            }
        }
    }
    if views.is_empty() {
        views.push(SystemView {
            id: "all".to_string(),
            label: "All".to_string(),
            q: None,
            sort: None,
            columns: None,
            declared: false,
        });
    }
    views
}

fn builtin_views(resource_id: &str) -> &'static [BuiltinView] {
    match resource_id {
        "targets" => &[
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "recently-seen",
                label: "Recently Seen",
                q: None,
                sort: Some("lastControllerRequestAt:DESC"),
                columns: None,
            },
            BuiltinView {
                // Answering inside the tenant's own poll window
                // (pollingTime + pollingOverdueTime). Oldest first: the top
                // of this list is what is about to fall out of it.
                id: "responding",
                label: "Responding",
                q: Some("pollState==responding"),
                sort: Some("lastControllerRequestAt:ASC"),
                columns: None,
            },
            BuiltinView {
                // The exact complement: overdue and never-polled together.
                // `!=` rather than `=in=(overdue,never)` because the server's
                // expression is total, so this is provably everything else —
                // a state added later lands here instead of falling through
                // both views.
                //
                // ASC puts NULL last, so a freshly imported fleet that has
                // never polled does not bury the devices that went quiet.
                id: "not-responding",
                label: "Not Responding",
                q: Some("pollState!=responding"),
                sort: Some("lastControllerRequestAt:ASC"),
                columns: None,
            },
            BuiltinView {
                id: "needs-attention",
                label: "Needs Attention",
                q: Some("updateStatus==error,updateStatus==pending"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "pending-update",
                label: "Pending Update",
                q: Some("updateStatus==pending"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "error",
                label: "Error",
                q: Some("updateStatus==error"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "unassigned",
                label: "Unassigned",
                q: Some("assignedDistributionSet==null"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "by-tag",
                label: "By Tag",
                q: None,
                sort: Some("tag:ASC"),
                columns: None,
            },
            BuiltinView {
                id: "by-group",
                label: "By Group",
                q: None,
                sort: Some("group:ASC"),
                columns: Some(&[
                    "controllerId",
                    "group",
                    "name",
                    "updateStatus",
                    "lastControllerRequestAt",
                ]),
            },
        ],
        "rollouts" => &[
            BuiltinView {
                id: "latest",
                label: "Latest",
                q: None,
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "running",
                label: "Running",
                q: Some("status==RUNNING"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "needs-attention",
                label: "Needs Attention",
                // The rollout enum has no ERROR: hitting the error threshold
                // pauses the rollout, and approval can be denied.
                q: Some("status==PAUSED,status==APPROVAL_DENIED"),
                sort: Some("lastModifiedAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "paused",
                label: "Paused",
                q: Some("status==PAUSED"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "finished",
                label: "Finished",
                q: Some("status==FINISHED"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "stopped",
                label: "Stopped",
                q: Some("status==STOPPED"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "draft-ready",
                label: "Draft/Ready",
                q: Some("status==READY,status==CREATING"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "recently-changed",
                label: "Recently Changed",
                q: None,
                sort: Some("lastModifiedAt:DESC"),
                columns: None,
            },
        ],
        "distribution-sets" => &[
            BuiltinView {
                id: "latest",
                label: "Latest",
                q: None,
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "complete",
                label: "Complete",
                q: Some("complete==true"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "incomplete",
                label: "Incomplete",
                q: Some("complete==false"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "assigned",
                label: "Assigned",
                q: Some("assigned==true"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "unassigned",
                label: "Unassigned",
                q: Some("assigned==false"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "recently-changed",
                label: "Recently Changed",
                q: None,
                sort: Some("lastModifiedAt:DESC"),
                columns: None,
            },
        ],
        "software-modules" => &[
            BuiltinView {
                id: "latest",
                label: "Latest",
                q: None,
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "with-artifacts",
                label: "With Artifacts",
                q: Some("artifactCount=gt=0"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "without-artifacts",
                label: "Without Artifacts",
                q: Some("artifactCount==0"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "recently-changed",
                label: "Recently Changed",
                q: None,
                sort: Some("lastModifiedAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "by-type",
                label: "By Type",
                q: None,
                sort: Some("type:ASC"),
                columns: None,
            },
        ],
        "actions" => &[
            BuiltinView {
                id: "active",
                label: "Active",
                q: Some("status==pending"),
                sort: Some("id:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: Some("id:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "failed",
                label: "Failed",
                q: Some("detailstatus==error"),
                sort: Some("id:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "finished",
                label: "Finished",
                q: Some("detailstatus==finished"),
                sort: Some("id:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "from-rollouts",
                label: "From rollouts",
                q: Some("rollout!=null"),
                sort: Some("id:DESC"),
                columns: None,
            },
        ],
        "incidents" => &[
            BuiltinView {
                id: "important",
                label: "Important",
                // Devices, API clients, and any server fault — console-side
                // noise is filtered out (see hawier_incident.kind).
                q: Some("kind=in=(ddi,mgmt),status=ge=500"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "ddi",
                label: "Devices (DDI)",
                q: Some("kind==ddi"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "mgmt",
                label: "API clients",
                q: Some("kind==mgmt"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "console",
                label: "Console",
                q: Some("kind=in=(console,ui)"),
                sort: None,
                columns: None,
            },
            BuiltinView {
                id: "server-faults",
                label: "Server 5xx",
                q: Some("status=ge=500"),
                sort: None,
                columns: None,
            },
        ],
        "roles" => &[
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: Some("name:ASC"),
                columns: None,
            },
            BuiltinView {
                // Roles nobody holds do nothing; usually a half-finished
                // setup rather than a deliberate one.
                id: "unheld",
                label: "Unheld",
                q: Some("holders==0"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
        ],
        "grants" => &[BuiltinView {
            id: "all",
            label: "All",
            q: None,
            sort: Some("username:ASC"),
            columns: None,
        }],
        "retention-rules" => &[
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: Some("name:ASC"),
                columns: None,
            },
            BuiltinView {
                // Rules that actually delete, as opposed to the ones still
                // only reporting what they would do.
                id: "armed",
                label: "Armed",
                q: Some("active==true;dryrun==false"),
                sort: Some("name:ASC"),
                columns: None,
            },
            BuiltinView {
                id: "dry-run",
                label: "Dry run",
                q: Some("dryrun==true"),
                sort: Some("name:ASC"),
                columns: None,
            },
            BuiltinView {
                id: "paused",
                label: "Paused",
                q: Some("active==false"),
                sort: Some("name:ASC"),
                columns: None,
            },
        ],
        "webhooks" => &[
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "active",
                label: "Active",
                q: Some("active==true"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "paused",
                label: "Paused",
                q: Some("active==false"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                // No secret means deliveries carry no signature, so the
                // receiver cannot tell they came from hawier.
                id: "unsigned",
                label: "Unsigned",
                q: Some("signed==false"),
                sort: Some("name:ASC"),
                columns: None,
            },
        ],
        "webhook-deliveries" => &[
            BuiltinView {
                id: "latest",
                label: "Latest",
                q: None,
                sort: Some("deliveredAt:DESC"),
                columns: None,
            },
            BuiltinView {
                // status 0 is "never got an answer"; anything outside 2xx is
                // a subscriber that is not receiving what it subscribed to.
                id: "failed",
                label: "Failed",
                q: Some("status=lt=200,status=ge=300"),
                sort: Some("deliveredAt:DESC"),
                columns: None,
            },
        ],
        "action-log" => &[
            BuiltinView {
                id: "latest",
                label: "Latest",
                q: None,
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "failed",
                label: "Failed",
                q: Some("status==error"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "changes",
                label: "Changes",
                q: Some("action=in=(create,edit,delete)"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
        ],
        "target-filters" => &[
            BuiltinView {
                id: "all",
                label: "All",
                q: None,
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "auto-assignment",
                label: "Auto Assignment",
                q: Some("autoAssignDistributionSet!=null"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "recently-used",
                label: "Recently Used",
                q: None,
                sort: Some("lastModifiedAt:DESC"),
                columns: None,
            },
            BuiltinView {
                id: "without-auto-assignment",
                label: "Without Auto Assignment",
                q: Some("autoAssignDistributionSet==null"),
                sort: Some("createdAt:DESC"),
                columns: None,
            },
        ],
        _ => &[BuiltinView {
            id: "all",
            label: "All",
            q: None,
            sort: None,
            columns: None,
        }],
    }
}

pub fn default_view_id(resource: &Resource) -> String {
    system_views(&resource.id)
        .first()
        .map(|view| view.id.clone())
        .unwrap_or_else(|| "all".to_string())
}

pub fn selected_columns(resource: &Resource, columns_csv: &str) -> Vec<String> {
    let mut columns = columns_csv
        .split(',')
        .map(str::trim)
        .filter(|column| resource.columns.iter().any(|c| c == column))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if columns.is_empty() {
        columns.clone_from(&resource.columns);
    }
    columns
}

pub fn build_payload(
    resource: &Resource,
    payload: &HashMap<String, String>,
    mode: PayloadMode,
) -> Result<Value, FormValidationError> {
    let Some(form) = resource.form else {
        let mut field_errors = HashMap::new();
        field_errors.insert(
            "_form".to_string(),
            format!(
                "{} does not support create/edit from this UI yet",
                resource.label
            ),
        );
        return Err(FormValidationError { field_errors });
    };

    validate_required(form, payload)?;
    let object = match form.kind {
        FormKind::Rollout => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            let ds: i64 = trimmed(payload, "distributionSetId").parse().map_err(|_| {
                let mut field_errors = HashMap::new();
                field_errors.insert(
                    "distributionSetId".to_string(),
                    "select a distribution set".to_string(),
                );
                FormValidationError { field_errors }
            })?;
            object.insert("distributionSetId".to_string(), json!(ds));
            object.insert(
                "targetFilterQuery".to_string(),
                json!(trimmed(payload, "targetFilterQuery")),
            );
            let groups: i64 = trimmed(payload, "amountGroups").parse().unwrap_or(1).max(1);
            object.insert("amountGroups".to_string(), json!(groups));
            let success = trimmed(payload, "successExpression");
            object.insert(
                "successCondition".to_string(),
                json!({
                    "condition": "THRESHOLD",
                    "expression": if success.is_empty() { "100".to_string() } else { success },
                }),
            );
            let error = trimmed(payload, "errorExpression");
            if !error.is_empty() {
                object.insert(
                    "errorCondition".to_string(),
                    json!({ "condition": "THRESHOLD", "expression": error }),
                );
            }
            let action_type = trimmed(payload, "type");
            if !action_type.is_empty() {
                object.insert("type".to_string(), json!(action_type));
            }
            insert_optional(&mut object, payload, "description");
            Value::Object(object)
        }
        FormKind::Target => {
            let mut object = Map::new();
            if matches!(mode, PayloadMode::Create) {
                object.insert(
                    "controllerId".to_string(),
                    json!(trimmed(payload, "controllerId")),
                );
            }
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            insert_optional(&mut object, payload, "description");
            insert_optional(&mut object, payload, "securityToken");
            insert_optional(&mut object, payload, "address");
            Value::Object(object)
        }
        FormKind::TargetFilter => json!({
            "name": trimmed(payload, "name"),
            "query": trimmed(payload, "query"),
        }),
        FormKind::DistributionSet => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            object.insert("version".to_string(), json!(trimmed(payload, "version")));
            object.insert("type".to_string(), json!(trimmed(payload, "type")));
            insert_optional(&mut object, payload, "description");
            object.insert(
                "requiredMigrationStep".to_string(),
                json!(checkbox(payload, "requiredMigrationStep")),
            );
            Value::Object(object)
        }
        FormKind::SoftwareModule => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            object.insert("version".to_string(), json!(trimmed(payload, "version")));
            object.insert("type".to_string(), json!(trimmed(payload, "type")));
            insert_optional(&mut object, payload, "vendor");
            insert_optional(&mut object, payload, "description");
            Value::Object(object)
        }
        FormKind::Tag => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            insert_optional(&mut object, payload, "description");
            insert_optional(&mut object, payload, "colour");
            Value::Object(object)
        }
        FormKind::Webhook => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            object.insert("url".to_string(), json!(trimmed(payload, "url")));
            // Blank means "leave the stored one alone" on update, and "no
            // signature" on create; the server never sends a secret back, so
            // an edit form cannot round-trip it.
            insert_optional(&mut object, payload, "secret");
            let mask = trimmed(payload, "eventMask");
            object.insert(
                "eventMask".to_string(),
                json!(if mask.is_empty() {
                    "*".to_string()
                } else {
                    mask
                }),
            );
            object.insert(
                "active".to_string(),
                json!(!trimmed(payload, "active").is_empty()),
            );
            Value::Object(object)
        }
        FormKind::Role => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            object.insert(
                "permissions".to_string(),
                json!(trimmed(payload, "permissions")),
            );
            insert_optional(&mut object, payload, "description");
            Value::Object(object)
        }
        FormKind::RetentionRule => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            object.insert("cron".to_string(), json!(trimmed(payload, "cron")));
            object.insert("filter".to_string(), json!(trimmed(payload, "filter")));
            if let Ok(age) = trimmed(payload, "minAgeMs").parse::<i64>() {
                object.insert("minAgeMs".to_string(), json!(age));
            }
            // An unticked box means false, and for a rule that deletes things
            // the safe reading of "unspecified" is the careful one — but the
            // form always sends it, so this is the operator's actual choice.
            object.insert(
                "dryRun".to_string(),
                json!(!trimmed(payload, "dryRun").is_empty()),
            );
            Value::Object(object)
        }
        FormKind::TypeWithKey => {
            let mut object = Map::new();
            object.insert("name".to_string(), json!(trimmed(payload, "name")));
            object.insert("key".to_string(), json!(trimmed(payload, "key")));
            insert_optional(&mut object, payload, "description");
            insert_optional(&mut object, payload, "colour");
            if let Ok(max) = trimmed(payload, "maxAssignments").parse::<i64>() {
                object.insert("maxAssignments".to_string(), json!(max));
            }
            Value::Object(object)
        }
    };

    match (form.kind, mode) {
        (
            FormKind::Target
            | FormKind::DistributionSet
            | FormKind::SoftwareModule
            | FormKind::Tag
            | FormKind::TypeWithKey,
            PayloadMode::Create,
        ) => Ok(Value::Array(vec![object])),
        _ => Ok(object),
    }
}

fn validate_required(
    form: FormSpec,
    payload: &HashMap<String, String>,
) -> Result<(), FormValidationError> {
    let mut field_errors = HashMap::new();
    for field in form.fields {
        if field.required && trimmed(payload, field.name).is_empty() {
            field_errors.insert(
                field.name.to_string(),
                format!("{} is required", field.label),
            );
        }
    }
    if field_errors.is_empty() {
        Ok(())
    } else {
        Err(FormValidationError { field_errors })
    }
}

fn trimmed(payload: &HashMap<String, String>, key: &str) -> String {
    payload
        .get(key)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn checkbox(payload: &HashMap<String, String>, key: &str) -> bool {
    matches!(
        payload.get(key).map(String::as_str),
        Some("on" | "true" | "1")
    )
}

fn insert_optional(object: &mut Map<String, Value>, payload: &HashMap<String, String>, key: &str) {
    let value = trimmed(payload, key);
    if !value.is_empty() {
        object.insert(key.to_string(), json!(value));
    }
}

/// Adopt what an extension declared.
///
/// Anything that would collide with a built-in is dropped rather than
/// merged: the console's own resources are not up for redefinition, and a
/// declaration that silently replaced one would be very hard to notice.
pub fn adopt(declaration: hawier_api::ext::ConsoleDeclaration) {
    let group_of = |name: &str| match name.trim().to_ascii_lowercase().as_str() {
        "inventory" => Group::Inventory,
        "deployment" => Group::Deployment,
        "repository" => Group::Repository,
        // Not silently dropped: an unrecognised group still shows up, in the
        // section where an operator looks for things they did not expect.
        _ => Group::Admin,
    };
    let convert = |view: hawier_api::ext::SurfaceView| SystemView {
        id: view.id,
        label: view.label,
        q: view.q,
        sort: view.sort,
        columns: view.columns,
        declared: true,
    };

    let mut resources = Vec::new();
    let mut views: Vec<(String, Vec<SystemView>)> = Vec::new();
    for surface in declaration.surfaces {
        views.push((
            surface.id.clone(),
            surface.views.into_iter().map(convert).collect(),
        ));
        resources.push(Resource {
            id: surface.id,
            label: surface.label,
            endpoint: surface.endpoint,
            group: group_of(&surface.group),
            id_field: surface.id_field,
            columns: surface.columns,
            default_sort: surface.default_sort,
            search_fields: surface.search_fields,
            // No create form: a declaration says how something is shown, and
            // writing needs a payload shape the console cannot guess.
            form: None,
            create_verified: false,
            row_href: None,
            icon: surface.icon,
        });
    }
    for extra in declaration.views {
        views.push((
            extra.resource,
            extra.views.into_iter().map(convert).collect(),
        ));
    }
    install(resources, views);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_target_as_array_payload() {
        let payload = HashMap::from([
            ("controllerId".to_string(), "dev01".to_string()),
            ("name".to_string(), "Device 01".to_string()),
            ("description".to_string(), "first target".to_string()),
        ]);
        let value =
            build_payload(&by_id("targets").unwrap(), &payload, PayloadMode::Create).unwrap();
        assert_eq!(
            value,
            json!([{
                "controllerId": "dev01",
                "name": "Device 01",
                "description": "first target"
            }])
        );
    }

    #[test]
    fn creates_rollout_as_object_payload() {
        let resource = by_id("rollouts").unwrap();
        let mut payload = HashMap::new();
        payload.insert("name".to_string(), "fleet".to_string());
        payload.insert("distributionSetId".to_string(), "7".to_string());
        payload.insert("targetFilterQuery".to_string(), "name==x*".to_string());
        payload.insert("amountGroups".to_string(), "3".to_string());
        payload.insert("errorExpression".to_string(), "40".to_string());
        let value = build_payload(&resource, &payload, PayloadMode::Create).unwrap();
        assert!(value.is_object(), "rollout create is a bare object");
        assert_eq!(value["distributionSetId"], 7);
        assert_eq!(value["amountGroups"], 3);
        assert_eq!(value["successCondition"]["expression"], "100");
        assert_eq!(value["errorCondition"]["expression"], "40");
    }

    #[test]
    fn creates_target_filter_as_object_payload() {
        let payload = HashMap::from([
            ("name".to_string(), "EMEA".to_string()),
            ("query".to_string(), "name==emea*".to_string()),
        ]);
        let value = build_payload(
            &by_id("target-filters").unwrap(),
            &payload,
            PayloadMode::Create,
        )
        .unwrap();
        assert_eq!(
            value,
            json!({
                "name": "EMEA",
                "query": "name==emea*"
            })
        );
    }

    #[test]
    fn creates_distribution_set_as_array_payload() {
        let payload = HashMap::from([
            ("name".to_string(), "MyDS".to_string()),
            ("version".to_string(), "1.0".to_string()),
            ("type".to_string(), "os".to_string()),
        ]);
        let value = build_payload(
            &by_id("distribution-sets").unwrap(),
            &payload,
            PayloadMode::Create,
        )
        .unwrap();
        assert_eq!(
            value,
            json!([{
                "name": "MyDS",
                "version": "1.0",
                "type": "os",
                "requiredMigrationStep": false
            }])
        );
    }

    #[test]
    fn creates_software_module_as_array_payload() {
        let payload = HashMap::from([
            ("name".to_string(), "MyOS".to_string()),
            ("version".to_string(), "1.0".to_string()),
            ("type".to_string(), "os".to_string()),
            ("vendor".to_string(), "Example Ltd.".to_string()),
        ]);
        let value = build_payload(
            &by_id("software-modules").unwrap(),
            &payload,
            PayloadMode::Create,
        )
        .unwrap();
        assert_eq!(
            value,
            json!([{
                "name": "MyOS",
                "version": "1.0",
                "type": "os",
                "vendor": "Example Ltd."
            }])
        );
    }

    #[test]
    fn creates_tag_and_type_as_array_payloads() {
        let payload = HashMap::from([
            ("name".to_string(), "eu-fleet".to_string()),
            ("colour".to_string(), "#ff0000".to_string()),
        ]);
        let value = build_payload(
            &by_id("target-tags").unwrap(),
            &payload,
            PayloadMode::Create,
        )
        .unwrap();
        assert_eq!(value, json!([{ "name": "eu-fleet", "colour": "#ff0000" }]));

        let payload = HashMap::from([
            ("name".to_string(), "Firmware".to_string()),
            ("key".to_string(), "fw".to_string()),
            ("maxAssignments".to_string(), "2".to_string()),
        ]);
        let value = build_payload(
            &by_id("software-module-types").unwrap(),
            &payload,
            PayloadMode::Create,
        )
        .unwrap();
        assert_eq!(
            value,
            json!([{ "name": "Firmware", "key": "fw", "maxAssignments": 2 }])
        );
    }

    #[test]
    fn updates_distribution_set_as_object_payload() {
        let payload = HashMap::from([
            ("name".to_string(), "MyDS".to_string()),
            ("version".to_string(), "1.0".to_string()),
            ("type".to_string(), "os".to_string()),
        ]);
        let value = build_payload(
            &by_id("distribution-sets").unwrap(),
            &payload,
            PayloadMode::Update,
        )
        .unwrap();
        assert_eq!(
            value,
            json!({
                "name": "MyDS",
                "version": "1.0",
                "type": "os",
                "requiredMigrationStep": false
            })
        );
    }
}
