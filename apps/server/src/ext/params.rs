//! Extension parameters: what a guest module reads its configuration from.
//!
//! A module gets no ambient anything — no filesystem, no sockets, no argv.
//! What it does get is a WASI environment, and that environment is this: the
//! `HAWIER_EXT_*` names, and nothing else the node knows. So "configuring an
//! extension" means deciding what goes in this map.
//!
//! Two sources feed it. The process environment is where a *deployment* pins
//! values that must match the machine it runs on — `HAWIER_EXT_FS_ROOT` has
//! to agree with a volume mount, and nothing an operator types in a console
//! should be able to break that. A file is where an *operator* configures the
//! rest, through the console, without a redeploy. The environment wins, and
//! where it wins the console says so out loud rather than accepting an edit
//! it knows is inert.
//!
//! The file is `KEY=value`, line-oriented, and it is that on purpose: these
//! values *are* the guest's environment, which is a flat `String -> String`
//! map. Any nested format would need an invented flattening that means
//! nothing to the guest and nothing to the operator — and the same file
//! already exists, in that same shape, as the `environment:` block of a
//! compose file.
//!
//! Nothing here is stored in the database. The database is shared by every
//! node; parameters are per node, because a sub node legitimately has a
//! different artifact root than the control node. A shared row would make two
//! nodes fight over one value, and would invite hot-reloading it — which the
//! host cannot do anyway: a module reads its environment when it is
//! instantiated, so a change applies when the node restarts, and the console
//! has to say that rather than imply otherwise.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Names the file may not set, whatever an operator types.
///
/// These choose *which code the node runs*, and that is a trust decision
/// rather than a parameter. The host grants a module's `http_out` allowlist
/// from the module's own manifest — the module names the hosts it may reach
/// and the host believes it. So an operator who could point
/// `HAWIER_EXT_STORAGE` at any `.wasm` on the box would, on the next restart,
/// be choosing code that grants itself whatever network access it likes.
/// Deploying a node stays the deployer's job.
///
/// They are still *shown* in the inventory: knowing which module is loaded is
/// exactly what the page is for.
const RESERVED: [&str; 8] = [
    "HAWIER_EXT_STORAGE",
    "HAWIER_EXT_EVENT_SINK",
    "HAWIER_EXT_RETENTION",
    "HAWIER_EXT_ROLLOUT",
    "HAWIER_EXT_CONSOLE",
    "HAWIER_EXT_AUTHZ",
    "HAWIER_EXT_DEADLINE_MS",
    "HAWIER_EXT_PARAMS_FILE",
];

/// The prefix that decides whether a name is an extension parameter at all.
pub const PREFIX: &str = "HAWIER_EXT_";

pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

/// Does this name hold a credential?
///
/// Deliberately wide: over-masking costs a operator one extra look at the
/// file, under-masking puts a secret key on a page and in an action log.
/// `_KEY` catches `HAWIER_EXT_S3_ACCESS_KEY` as well as `..._SECRET_KEY` —
/// an access key is half a credential, and the pair is one secret.
pub fn is_secret(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.contains("SECRET")
        || name.contains("PASSWORD")
        || name.contains("TOKEN")
        || name.contains("CREDENTIAL")
        || name.ends_with("_KEY")
}

/// Where an effective value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The process environment. Pinned by the deployment; not editable here.
    Environment,
    /// The parameters file. Editable through the console.
    File,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Environment => "environment",
            Origin::File => "file",
        }
    }
}

/// One effective parameter.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub value: String,
    pub origin: Origin,
    /// The file also carries a line for this name. With
    /// `origin == Environment` that means the line is being ignored, and the
    /// console has to say so — an operator editing into invisibility is the
    /// failure this field exists to prevent.
    pub in_file: bool,
}

impl Param {
    pub fn secret(&self) -> bool {
        is_secret(&self.name)
    }

    /// Editable through the console.
    ///
    /// Exactly "the file is where this value comes from". It is not "the file
    /// also has a line for it": a value the environment supplies is pinned
    /// whether or not the file mentions it, and offering an edit for one
    /// would write a line the node then ignores — which is the failure the
    /// whole origin/pinning apparatus exists to prevent. A name that is set
    /// nowhere is not in this list at all; new ones arrive through the add
    /// form.
    pub fn editable(&self) -> bool {
        self.origin == Origin::File
    }
}

/// What became of the file this node was pointed at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FileState {
    /// No path configured: the feature is off.
    #[default]
    Unconfigured,
    /// Configured, and there is nothing there yet. Not an error — a save
    /// creates it.
    Absent,
    Ok,
    /// Configured and there, but this node cannot read it right now.
    Unreadable(String),
    /// Configured, read, and not a parameters file. Only reachable at
    /// startup, where it is fatal.
    Malformed(String),
}

impl FileState {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileState::Unconfigured => "unconfigured",
            FileState::Absent => "absent",
            FileState::Ok => "ok",
            FileState::Unreadable(_) => "unreadable",
            FileState::Malformed(_) => "malformed",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            FileState::Unreadable(why) | FileState::Malformed(why) => why.clone(),
            _ => String::new(),
        }
    }
}

/// One line of the file, kept verbatim so a rewrite does not eat comments.
///
/// An operator whose comments vanished the first time they used the console
/// stops using the console — and the alternative, a serde round-trip through
/// a nested format, drops them by construction.
#[derive(Debug, Clone)]
enum Line {
    /// A comment or a blank line, exactly as written.
    Verbatim(String),
    Entry {
        name: String,
        value: String,
    },
}

/// Everything this node knows about its extension parameters.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub path: Option<PathBuf>,
    pub state: FileState,
    /// Effective values, after the environment has beaten the file.
    params: BTreeMap<String, Param>,
    lines: Vec<Line>,
}

impl Resolved {
    /// Read the environment and the file, and decide what is in force.
    ///
    /// Never fails. A file that cannot be read leaves the node running on the
    /// environment alone with the reason recorded — the inventory endpoint
    /// must answer even when the file is gone, or a mount that disappeared
    /// would take the settings page down with it. Only the *startup* caller
    /// treats `Malformed` as fatal.
    pub fn load(path: Option<&Path>) -> Self {
        let env: BTreeMap<String, String> = std::env::vars()
            .filter(|(name, _)| name.starts_with(PREFIX))
            .collect();
        Self::from_parts(path, env)
    }

    /// The same resolution over an explicit environment rather than the
    /// process's own. Tests resolve this way: the environment beats the file
    /// here, and a test that exported a `HAWIER_EXT_*` value would decide
    /// what every node started later in the same process resolves.
    pub fn from_parts(path: Option<&Path>, env: BTreeMap<String, String>) -> Self {
        let mut resolved = Resolved {
            path: path.map(Path::to_path_buf),
            state: FileState::Unconfigured,
            params: BTreeMap::new(),
            lines: Vec::new(),
        };

        let mut from_file: BTreeMap<String, String> = BTreeMap::new();
        if let Some(path) = path {
            match std::fs::read_to_string(path) {
                Ok(text) => match parse(&text) {
                    Ok(lines) => {
                        for line in &lines {
                            if let Line::Entry { name, value } = line {
                                from_file.insert(name.clone(), value.clone());
                            }
                        }
                        resolved.lines = lines;
                        resolved.state = FileState::Ok;
                    }
                    Err(why) => resolved.state = FileState::Malformed(why),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    resolved.state = FileState::Absent;
                }
                Err(error) => resolved.state = FileState::Unreadable(error.to_string()),
            }
        }

        for (name, value) in &from_file {
            resolved.params.insert(
                name.clone(),
                Param {
                    name: name.clone(),
                    value: value.clone(),
                    origin: Origin::File,
                    in_file: true,
                },
            );
        }
        // The environment wins: a deployment must be able to pin a value that
        // has to match the machine, and an operator must not be able to break
        // artifact serving from a settings page.
        for (name, value) in env {
            let in_file = from_file.contains_key(&name);
            resolved.params.insert(
                name.clone(),
                Param {
                    name,
                    value,
                    origin: Origin::Environment,
                    in_file,
                },
            );
        }
        resolved
    }

    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.params.values()
    }

    pub fn get(&self, name: &str) -> Option<&Param> {
        self.params.get(name)
    }

    /// What a guest actually receives.
    ///
    /// NARROW: this is one flat map handed to every module, so each module
    /// sees every other module's parameters — `hawier-ext-fs` receives
    /// `HAWIER_EXT_S3_SECRET_KEY` if S3 is also configured. Excluding the
    /// reserved names narrows it (a module no longer learns which modules are
    /// loaded and from where) but does not close it. Closing it means
    /// per-point sections in the file, which changes what already-shipped
    /// modules receive. See EXTENSIONS.md.
    pub fn for_guest(&self) -> Vec<(String, String)> {
        self.params
            .values()
            .filter(|param| !is_reserved(&param.name))
            .map(|param| (param.name.clone(), param.value.clone()))
            .collect()
    }

    /// Names whose effective value differs from this snapshot's.
    ///
    /// Compares *effective* values, after precedence: a file line the
    /// environment shadows changes nothing, and reporting it would teach an
    /// operator to ignore the indicator.
    pub fn drift_from(&self, started_with: &Resolved) -> Vec<String> {
        let mut changed = Vec::new();
        for (name, param) in &self.params {
            match started_with.params.get(name) {
                Some(before) if before.value == param.value => {}
                _ => changed.push(name.clone()),
            }
        }
        for name in started_with.params.keys() {
            if !self.params.contains_key(name) {
                changed.push(name.clone());
            }
        }
        changed.sort();
        changed.dedup();
        changed
    }

    /// Rewrite the file with one name set or removed, keeping everything else
    /// — including comments, blank lines and order — exactly as it was.
    ///
    /// Written to a temporary file in the same directory and renamed, so a
    /// crash midway leaves the old file rather than half of a new one, and
    /// `0600` because this can hold credentials.
    pub fn write(&self, name: &str, value: Option<&str>) -> std::io::Result<()> {
        let path = self.path.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this node has no extension parameters file",
            )
        })?;

        let mut lines = self.lines.clone();
        match value {
            Some(value) => {
                let existing = lines
                    .iter_mut()
                    .find(|line| matches!(line, Line::Entry { name: n, .. } if n == name));
                match existing {
                    Some(line) => {
                        *line = Line::Entry {
                            name: name.to_string(),
                            value: value.to_string(),
                        }
                    }
                    None => lines.push(Line::Entry {
                        name: name.to_string(),
                        value: value.to_string(),
                    }),
                }
            }
            None => lines.retain(|line| !matches!(line, Line::Entry { name: n, .. } if n == name)),
        }

        let mut text = String::new();
        for line in &lines {
            match line {
                Line::Verbatim(raw) => text.push_str(raw),
                // Always the quoted form: a value the console wrote must come
                // back byte-exact, and a secret key can end in whitespace.
                Line::Entry { name, value } => {
                    text.push_str(name);
                    text.push('=');
                    text.push_str(&quote(value));
                }
            }
            text.push('\n');
        }

        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = directory.join(format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("params")
        ));
        {
            let mut file = std::fs::File::create(&temporary)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temporary, path)
    }
}

fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn unquote(raw: &str) -> String {
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            // An unknown escape is the two characters that were written: the
            // file may hold a Windows path, and eating the backslash would
            // change a value silently.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn parse(text: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lines.push(Line::Verbatim(raw.to_string()));
            continue;
        }
        // The first `=` splits. Everything after it is value, `=` included:
        // base64 credentials end in `=`.
        let Some((name, value)) = raw.split_once('=') else {
            return Err(format!("line {number}: expected NAME=value"));
        };
        let name = name.trim().to_string();
        if !name.starts_with(PREFIX)
            || !name[PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            || name.len() == PREFIX.len()
        {
            return Err(format!(
                "line {number}: '{name}' is not an extension parameter — a name \
                 outside {PREFIX}[A-Z0-9_]+ would never reach a module, and \
                 would sit here looking configured"
            ));
        }
        if is_reserved(&name) {
            return Err(format!(
                "line {number}: '{name}' selects which code the node runs and \
                 cannot be set from this file; it belongs to the deployment"
            ));
        }
        if let Some(first) = seen.get(&name) {
            // Not last-wins: this file can hold credentials, and an ambiguous
            // credentials file must not boot silently on one of them.
            return Err(format!(
                "line {number}: '{name}' is already set on line {first}"
            ));
        }
        seen.insert(name.clone(), number);

        let value = value.trim();
        // A `#` after the `=` is part of the value — S3 secret keys contain
        // `#`, `+`, `/` and `=`. Only a whole-line comment is a comment.
        let value = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            unquote(value)
        } else {
            value.to_string()
        };
        lines.push(Line::Entry { name, value });
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> BTreeMap<String, String> {
        parse(text)
            .expect("valid")
            .into_iter()
            .filter_map(|line| match line {
                Line::Entry { name, value } => Some((name, value)),
                Line::Verbatim(_) => None,
            })
            .collect()
    }

    #[test]
    fn reads_plain_entries() {
        let map = parsed("HAWIER_EXT_S3_BUCKET=firmware\nHAWIER_EXT_FS_ROOT=/artifacts\n");
        assert_eq!(map["HAWIER_EXT_S3_BUCKET"], "firmware");
        assert_eq!(map["HAWIER_EXT_FS_ROOT"], "/artifacts");
    }

    #[test]
    fn the_first_equals_splits_and_the_rest_is_value() {
        // Base64 credentials end in '=', and a value may contain more.
        let map = parsed("HAWIER_EXT_S3_SECRET_KEY=abc/def+ghi==\n");
        assert_eq!(map["HAWIER_EXT_S3_SECRET_KEY"], "abc/def+ghi==");
    }

    #[test]
    fn a_hash_after_the_equals_is_value_not_a_comment() {
        let map = parsed("HAWIER_EXT_S3_SECRET_KEY=pa#ss\n# a real comment\n");
        assert_eq!(map["HAWIER_EXT_S3_SECRET_KEY"], "pa#ss");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn quoted_values_keep_their_whitespace() {
        let map = parsed("HAWIER_EXT_S3_REGION=\"  eu-central-1  \"\n");
        assert_eq!(map["HAWIER_EXT_S3_REGION"], "  eu-central-1  ");
    }

    #[test]
    fn escapes_round_trip() {
        let original = "a\\b\"c\nd\te";
        let text = format!("HAWIER_EXT_S3_BUCKET={}\n", quote(original));
        assert_eq!(parsed(&text)["HAWIER_EXT_S3_BUCKET"], original);
    }

    #[test]
    fn an_unknown_escape_is_left_alone() {
        // A Windows path must not lose its separator to a silent unescape.
        let map = parsed("HAWIER_EXT_FS_ROOT=\"C:\\data\"\n");
        assert_eq!(map["HAWIER_EXT_FS_ROOT"], "C:\\data");
    }

    #[test]
    fn a_name_without_the_prefix_is_refused_with_its_line() {
        let error = parse("# fine\nPATH=/usr/bin\n").unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(error.contains("PATH"), "{error}");
    }

    #[test]
    fn a_reserved_name_is_refused() {
        let error = parse("HAWIER_EXT_STORAGE=/tmp/evil.wasm\n").unwrap_err();
        assert!(error.contains("HAWIER_EXT_STORAGE"), "{error}");
        assert!(error.contains("which code the node runs"), "{error}");
    }

    #[test]
    fn a_duplicate_name_names_both_lines() {
        // Last-wins on a credentials file means booting on the wrong secret
        // with nothing said about it.
        let error = parse("HAWIER_EXT_S3_BUCKET=a\n\nHAWIER_EXT_S3_BUCKET=b\n").unwrap_err();
        assert!(error.contains("line 3"), "{error}");
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn the_environment_beats_the_file() {
        let directory = std::env::temp_dir().join(format!("hawier-params-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("env-wins.env");
        std::fs::write(
            &path,
            "HAWIER_EXT_FS_ROOT=/from-file\nHAWIER_EXT_S3_BUCKET=from-file\n",
        )
        .unwrap();

        let env = BTreeMap::from([("HAWIER_EXT_FS_ROOT".to_string(), "/pinned".to_string())]);
        let resolved = Resolved::from_parts(Some(&path), env);

        let pinned = resolved.get("HAWIER_EXT_FS_ROOT").unwrap();
        assert_eq!(pinned.value, "/pinned");
        assert_eq!(pinned.origin, Origin::Environment);
        assert!(pinned.in_file, "the shadowed line must still be visible");
        assert!(!pinned.editable(), "a pinned value has no editable form");

        let free = resolved.get("HAWIER_EXT_S3_BUCKET").unwrap();
        assert_eq!(free.origin, Origin::File);
        assert!(free.editable());
        std::fs::remove_file(&path).ok();
    }

    /// A value the environment supplies is pinned even when the file has
    /// never heard of it.
    ///
    /// The first version of `editable` asked "is it also in the file?", which
    /// made every environment-only value look editable — the deployed node
    /// offered a form for HAWIER_EXT_FS_ROOT, and saving it would have
    /// written a line the node then ignores.
    #[test]
    fn an_environment_value_is_pinned_even_when_the_file_lacks_it() {
        let env = BTreeMap::from([("HAWIER_EXT_FS_ROOT".to_string(), "/artifacts".to_string())]);
        let resolved = Resolved::from_parts(None, env);
        let param = resolved.get("HAWIER_EXT_FS_ROOT").unwrap();
        assert_eq!(param.origin, Origin::Environment);
        assert!(!param.in_file);
        assert!(!param.editable(), "pinned by the environment either way");
    }

    #[test]
    fn a_guest_never_receives_a_reserved_name() {
        let env = BTreeMap::from([
            ("HAWIER_EXT_STORAGE".to_string(), "/lib/fs.wasm".to_string()),
            ("HAWIER_EXT_FS_ROOT".to_string(), "/artifacts".to_string()),
        ]);
        let resolved = Resolved::from_parts(None, env);
        let names: Vec<String> = resolved.for_guest().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["HAWIER_EXT_FS_ROOT"]);
        // But the inventory still knows it: that is what the page shows.
        assert!(resolved.get("HAWIER_EXT_STORAGE").is_some());
    }

    #[test]
    fn secrets_are_recognised_widely() {
        for name in [
            "HAWIER_EXT_S3_SECRET_KEY",
            "HAWIER_EXT_S3_ACCESS_KEY",
            "HAWIER_EXT_SOMETHING_TOKEN",
            "HAWIER_EXT_DB_PASSWORD",
            "HAWIER_EXT_GCP_CREDENTIALS",
        ] {
            assert!(is_secret(name), "{name} must be treated as a secret");
        }
        for name in [
            "HAWIER_EXT_FS_ROOT",
            "HAWIER_EXT_S3_BUCKET",
            "HAWIER_EXT_S3_ENDPOINT",
            "HAWIER_EXT_S3_REGION",
            "HAWIER_EXT_S3_PATH_STYLE",
            "HAWIER_EXT_S3_PRESIGN_TTL_SECS",
            "HAWIER_EXT_S3_SIGNER_URL",
        ] {
            assert!(!is_secret(name), "{name} is not a credential");
        }
    }

    #[test]
    fn a_rewrite_keeps_comments_blank_lines_and_order() {
        let directory =
            std::env::temp_dir().join(format!("hawier-params-w-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("rewrite.env");
        std::fs::write(
            &path,
            "# artifact storage\nHAWIER_EXT_FS_ROOT=/artifacts\n\n# s3\nHAWIER_EXT_S3_BUCKET=old\n",
        )
        .unwrap();

        let resolved = Resolved::from_parts(Some(&path), BTreeMap::new());
        resolved.write("HAWIER_EXT_S3_BUCKET", Some("new")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# artifact storage"), "{text}");
        assert!(text.contains("# s3"), "{text}");
        assert!(text.contains("HAWIER_EXT_S3_BUCKET=\"new\""), "{text}");
        assert!(!text.contains("old"), "{text}");
        assert!(
            text.find("HAWIER_EXT_FS_ROOT").unwrap() < text.find("HAWIER_EXT_S3_BUCKET").unwrap(),
            "order changed: {text}"
        );

        // And a value the console wrote reads back byte-exact.
        let again = Resolved::from_parts(Some(&path), BTreeMap::new());
        assert_eq!(again.get("HAWIER_EXT_S3_BUCKET").unwrap().value, "new");

        again.write("HAWIER_EXT_S3_BUCKET", None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("HAWIER_EXT_S3_BUCKET"), "{text}");
        assert!(text.contains("# s3"), "removing a value ate its comment");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn drift_compares_effective_values_not_file_lines() {
        let before = Resolved::from_parts(
            None,
            BTreeMap::from([("HAWIER_EXT_S3_BUCKET".to_string(), "a".to_string())]),
        );
        let same = Resolved::from_parts(
            None,
            BTreeMap::from([("HAWIER_EXT_S3_BUCKET".to_string(), "a".to_string())]),
        );
        assert!(same.drift_from(&before).is_empty());

        let changed = Resolved::from_parts(
            None,
            BTreeMap::from([("HAWIER_EXT_S3_BUCKET".to_string(), "b".to_string())]),
        );
        assert_eq!(changed.drift_from(&before), ["HAWIER_EXT_S3_BUCKET"]);

        // A removed parameter is a change too.
        let removed = Resolved::from_parts(None, BTreeMap::new());
        assert_eq!(removed.drift_from(&before), ["HAWIER_EXT_S3_BUCKET"]);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let resolved =
            Resolved::from_parts(Some(Path::new("/nonexistent/hawier.env")), BTreeMap::new());
        assert_eq!(resolved.state, FileState::Absent);
        assert!(resolved.params().next().is_none());
    }
}
