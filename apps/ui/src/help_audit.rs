//! The guide is part of the interface, so the checks that it covers every
//! word the console prints belong with the build, not with a running stack.
//!
//! These read `docs/` and `../../libs/api/src/codes.rs` off disk and need
//! neither a node nor a database, so they are unit tests: `make test` runs
//! them, and they do not pull the whole wasm runtime into a test binary the
//! way an integration target would. The ones that need a rendered page stay
//! in `tests/it/ui_audit.rs`.
//!
//! Cargo runs unit tests with the package root as the working directory —
//! the same as an integration test — so the relative paths below hold.

#[cfg(test)]
mod tests {
    use crate::resources;

    fn docs_dir() -> std::path::PathBuf {
        std::path::PathBuf::from("docs")
    }

    fn doc_files() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).expect("docs directory") {
                let path = entry.expect("doc entry").path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else if path.extension().is_some_and(|ext| ext == "md") {
                    let id = path
                        .strip_prefix(root)
                        .expect("under docs")
                        .with_extension("")
                        .to_string_lossy()
                        .to_string();
                    out.push((id, std::fs::read_to_string(&path).expect("doc file")));
                }
            }
        }
        let root = docs_dir();
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    /// Anything an operator can point at that the registry declares: every list,
    /// every view, every column, every bulk operation, every form field. This is
    /// the test that makes "cover everything" a property of the build rather than
    /// a promise.
    #[test]
    fn every_declared_thing_is_documented() {
        let mut missing: Vec<String> = Vec::new();

        for resource in resources::all() {
            let page = crate::help::page_key(Some(&resource.id));
            if page == crate::help::UNKNOWN_RESOURCE {
                missing.push(format!("resource {} has no chapter", resource.id));
            }
            for view in resources::system_views(&resource.id) {
                let key = crate::help::view_key(&resource.id, &view.id);
                if key == crate::help::UNKNOWN_VIEW {
                    missing.push(format!("view {}?view={} -> {key}", resource.id, view.id));
                }
            }
            for column in &resource.columns {
                let key = crate::help::field_key(&resource.id, column);
                if key == crate::help::UNKNOWN_FIELD {
                    missing.push(format!("column {}.{column} -> {key}", resource.id));
                }
            }
            for action in resources::bulk_actions(&resource.id) {
                let key = crate::help::bulk_key(action.id);
                if key == crate::help::UNKNOWN_FIELD {
                    missing.push(format!("bulk {}.{} -> {key}", resource.id, action.id));
                }
            }
            if let Some(form) = resource.form {
                for field in form.fields {
                    let key = crate::help::form_field_key(form.kind.help_topic(), field.name);
                    if key == crate::help::UNKNOWN_FIELD {
                        missing.push(format!(
                            "form field {}.{} -> {key}",
                            resource.id, field.name
                        ));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{} declared things have no section in the manual:\n{}",
            missing.len(),
            missing.join("\n")
        );
    }

    /// Every word the server can put in a status column has a glossary entry.
    /// Variants are scraped out of the shared codes table, so a new one on the
    /// server side fails here rather than reaching an operator unexplained.
    #[test]
    fn every_status_value_is_in_the_glossary() {
        let source =
            std::fs::read_to_string("../../libs/api/src/codes.rs").expect("the shared enum codes");
        let mut missing = Vec::new();
        for line in source.lines() {
            let line = line.trim();
            // `Variant = 3,` inside an int_enum! block.
            let Some((name, rest)) = line.split_once(" = ") else {
                continue;
            };
            if !rest
                .trim_end_matches(',')
                .chars()
                .all(|c| c.is_ascii_digit())
            {
                continue;
            }
            if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                continue;
            }
            let key = format!("glossary#{}", crate::help::slug(name));
            if !crate::help::has(&key) {
                missing.push(format!("{name} -> {key}"));
            }
        }
        // Plus the words the console itself invents for a badge.
        for word in ["ok", "success", "failed", "cancelled", "pending", "overdue"] {
            let key = format!("glossary#{}", crate::help::slug(word));
            if !crate::help::has(&key) {
                missing.push(format!("{word} -> {key}"));
            }
        }
        // Every word in the colour table is a word an operator will meet.
        for state in crate::help::statuses() {
            let key = format!("glossary#{}", crate::help::slug(state.word));
            if !crate::help::has(&key) {
                missing.push(format!("{} -> {key}", state.word));
            }
        }
        assert!(
            missing.is_empty(),
            "status words with no glossary entry:\n{}",
            missing.join("\n")
        );
    }

    /// A link inside the manual must still land somewhere: either on a section
    /// that exists, or on a console route that exists.
    #[test]
    fn internal_help_links_resolve() {
        let ids: Vec<String> = resources::all().into_iter().map(|r| r.id).collect();
        let static_paths = [
            "/",
            "/settings",
            "/search",
            "/help",
            "/login",
            "/incidents",
            "/action-log",
        ];
        let mut broken = Vec::new();
        for topic in crate::help::manual().topics() {
            for target in link_targets(topic) {
                if let Some(key) = target.strip_prefix("/help/") {
                    if !crate::help::has(key) && crate::help::topic(key).is_none() {
                        broken.push(format!("{}: /help/{key}", topic.id));
                    }
                    continue;
                }
                let path = target.split(['?', '#']).next().unwrap_or(&target);
                let known =
                    static_paths.contains(&path) || ids.iter().any(|id| path == format!("/{id}"));
                if !known {
                    broken.push(format!("{}: {target}", topic.id));
                }
            }
        }
        assert!(
            broken.is_empty(),
            "the manual links to {} places that do not exist:\n{}",
            broken.len(),
            broken.join("\n")
        );
    }

    /// Link targets straight out of the sources — the parsed model keeps only the
    /// resolved href, and this is about what a writer typed.
    fn link_targets(topic: &crate::help::Topic) -> Vec<String> {
        let path = docs_dir().join(format!("{}.md", topic.id));
        let source = std::fs::read_to_string(path).expect("doc source");
        let mut out = Vec::new();
        for piece in source.split("](").skip(1) {
            if let Some(target) = piece.split(')').next() {
                if target.starts_with('#') {
                    out.push(format!("/help/{}{target}", topic.id));
                } else {
                    out.push(target.to_string());
                }
            }
        }
        out
    }

    /// The sources stay inside the subset the parser understands, and every file
    /// on disk is a topic the console actually serves.
    #[test]
    fn docs_are_plain_markdown() {
        let files = doc_files();
        assert!(!files.is_empty(), "the manual has no sources");
        for (id, source) in &files {
            assert!(
                crate::help::topic(id).is_some(),
                "docs/{id}.md is not in the topic table — the console will never serve it"
            );
            assert!(
                source.starts_with("# "),
                "docs/{id}.md does not start with its title"
            );
            // The manual is interface: the upstream project's name does not belong
            // in it, and neither does raw markup or a table the parser drops.
            for word in ["hawkBit", "hawier-ui"] {
                assert!(
                    !source.contains(word),
                    "docs/{id}.md prints '{word}' — the manual speaks in this product's own terms"
                );
            }
            let mut fenced = false;
            for (number, line) in source.lines().enumerate() {
                let line = line.trim();
                if line == "```" {
                    fenced = !fenced;
                    continue;
                }
                if !fenced {
                    // Emphasis is `**bold**` only. A lone asterisk in prose used to
                    // reach the panel as a literal `*responding*`.
                    let mut prose = String::new();
                    for (index, piece) in line.split('`').enumerate() {
                        if index % 2 == 0 {
                            prose.push_str(piece);
                        }
                    }
                    assert!(
                        !prose.replace("**", "").contains('*'),
                        "docs/{id}.md:{}: a lone `*` renders literally — write **bold**",
                        number + 1
                    );
                }
                assert!(
                    !line.starts_with('|'),
                    "docs/{id}.md:{}: tables are not part of the subset — use `term :: definition`",
                    number + 1
                );
                assert!(
                    !line.starts_with("!["),
                    "docs/{id}.md:{}: images are not part of the subset",
                    number + 1
                );
                assert!(
                    !line.starts_with("### "),
                    "docs/{id}.md:{}: only one heading level — `##` makes a section",
                    number + 1
                );
            }
        }
        assert_eq!(
            files.len(),
            crate::help::manual().topics().len(),
            "the topic table and docs/ disagree about how many topics exist"
        );
    }

    /// A chapter nobody can reach is a chapter nobody reads.
    #[test]
    fn no_orphan_help_topics() {
        let mut linked: std::collections::HashSet<String> = std::collections::HashSet::new();
        for topic in crate::help::manual().topics() {
            for target in link_targets(topic) {
                if let Some(key) = target.strip_prefix("/help/") {
                    linked.insert(key.split('#').next().unwrap_or(key).to_string());
                }
            }
        }
        // The table of contents lists every topic, so reachability is about the
        // prose: a chapter no other chapter mentions is one an operator only finds
        // by reading the index top to bottom.
        let mut lonely: Vec<&str> = crate::help::manual()
            .topics()
            .iter()
            .map(|topic| topic.id)
            .filter(|id| !linked.contains(*id))
            .collect();
        lonely.retain(|id| !id.starts_with("fields/") && !id.starts_with("forms/"));
        assert!(
            lonely.is_empty(),
            "no other chapter links to: {}",
            lonely.join(", ")
        );
    }

    /// The four state enums are what a list prints as a badge, so every one of
    /// their variants needs a colour. Action *types* are exempt on purpose: the
    /// console prints `forced` and `soft` as plain text.
    #[test]
    fn every_state_the_server_can_report_has_a_colour() {
        let source =
            std::fs::read_to_string("../../libs/api/src/codes.rs").expect("the shared enum codes");
        let states = [
            "ActionStatus",
            "RolloutStatus",
            "RolloutGroupStatus",
            "TargetUpdateStatus",
        ];
        let mut current = "";
        let mut missing = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_suffix(" {") {
                if states.contains(&name) {
                    current = states.iter().find(|s| **s == name).unwrap();
                } else if !name.is_empty() {
                    current = "";
                }
            }
            if current.is_empty() {
                continue;
            }
            let Some((name, rest)) = trimmed.split_once(" = ") else {
                continue;
            };
            if !rest
                .trim_end_matches(',')
                .chars()
                .all(|c| c.is_ascii_digit())
            {
                continue;
            }
            if crate::help::status(name).is_none() {
                missing.push(format!("{current}::{name}"));
            }
        }
        assert!(
            missing.is_empty(),
            "states the console would print grey because nobody gave them a colour:\n{}",
            missing.join("\n")
        );
    }

    /// The glossary is the legend for the lists, so it has to print the same
    /// spelling: `in_sync`, not "in sync".
    #[test]
    fn the_glossary_prints_the_words_the_lists_print() {
        let glossary = crate::help::topic("glossary").expect("a glossary");
        let mut wrong = Vec::new();
        for state in crate::help::statuses() {
            let anchor = crate::help::slug(state.word);
            match glossary
                .sections
                .iter()
                .find(|section| section.anchor == anchor)
            {
                Some(section) if section.title != state.word => wrong.push(format!(
                    "'{}' is headed '{}' — the lists print '{}'",
                    anchor, section.title, state.word
                )),
                Some(_) => {}
                None => wrong.push(format!("'{}' has no entry", state.word)),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Widening the table must not restyle a list that already worked.
    #[test]
    fn the_colours_the_lists_already_had_do_not_move() {
        for (word, class) in [
            ("ok", "ok"),
            ("success", "ok"),
            ("finished", "ok"),
            ("running", "ok"),
            ("ready", "ok"),
            ("pending", "warn"),
            ("paused", "warn"),
            ("warning", "warn"),
            ("creating", "warn"),
            ("error", "error"),
            ("failed", "error"),
            ("cancelled", "error"),
            ("canceled", "error"),
            ("overdue", "error"),
            ("responding", "ok"),
        ] {
            assert_eq!(
                crate::view::status_class(word),
                class,
                "{word} changed colour"
            );
        }
    }
}
