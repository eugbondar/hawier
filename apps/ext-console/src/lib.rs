//! hawier console extension: extra nodes and views in the tree.
//!
//! An extension declares how something is *presented* — a tree node over a
//! mgmt endpoint the node already serves, and named queries under it. It does
//! not supply the data: serving that would need an extension point which
//! answers HTTP, and there is none. So a surface is a saved query with a
//! name, promoted to a first-class place in the navigation.
//!
//! What ships here is a worked example rather than a policy: the views a
//! particular deployment tends to want, expressed the way that deployment
//! would express them. Replace this module to get a different tree.
//!
//! It used to declare a whole `fleet-health` node. That was the wrong shape:
//! "which devices are answering" is the product's question, not a
//! deployment's, and it now lives in the console's own Targets resource,
//! where the server can compute it against the tenant's polling
//! configuration. What is left here is what an extension is actually for —
//! queries only this deployment cares about. `ConsoleSurface` stays in the
//! ABI and is exercised by the console's own tests; nothing here needs it
//! today.

use hawier_api::ext::{
    points, ConsoleDeclaration, ConsoleViews, ExecutionSite, ExtManifest, ExtRequest, ExtResponse,
    SurfaceView,
};

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-console".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![points::CONSOLE_SURFACES.to_string()],
        // Read once, when the console starts.
        sites: vec![ExecutionSite::Control],
        capabilities: Vec::new(),
        deadline_ms: 1_000,
    }
}

fn call(request: ExtRequest) -> ExtResponse {
    if request.point != points::CONSOLE_SURFACES {
        return ExtResponse::Err {
            message: format!("hawier-ext-console does not implement {}", request.point),
        };
    }
    match serde_json::to_value(declaration()) {
        Ok(payload) => ExtResponse::Ok { payload },
        Err(error) => ExtResponse::Err {
            message: format!("cannot encode the declaration: {error}"),
        },
    }
}

fn view(id: &str, label: &str, q: Option<&str>, sort: Option<&str>) -> SurfaceView {
    SurfaceView {
        id: id.to_string(),
        label: label.to_string(),
        q: q.map(str::to_string),
        sort: sort.map(str::to_string),
        columns: None,
    }
}

fn declaration() -> ConsoleDeclaration {
    ConsoleDeclaration {
        // No surface today. The API is intact and documented; this deployment
        // simply has nothing that deserves a node of its own.
        surfaces: Vec::new(),
        views: vec![
            ConsoleViews {
                resource: "targets".to_string(),
                views: vec![
                    // Tagging is a deployment's own convention, which is
                    // exactly the kind of thing that belongs in a module
                    // rather than in the console's table.
                    view(
                        "field-trial",
                        "Field Trial",
                        Some("tag==\"field-trial\""),
                        Some("lastControllerRequestAt:DESC"),
                    ),
                    view(
                        "never-polled",
                        "Never Polled",
                        Some("pollState==never"),
                        Some("createdAt:DESC"),
                    ),
                ],
            },
            ConsoleViews {
                resource: "rollouts".to_string(),
                // Not "needs-attention": the console has a built-in view with
                // that id, and a declared view that collides with a built-in
                // is dropped — silently, until this project made it log.
                views: vec![view(
                    "stalled",
                    "Stalled",
                    Some("status==paused"),
                    Some("lastModifiedAt:DESC"),
                )],
            },
        ],
    }
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;
    // Only the round-trip test needs it: nothing declares a surface today.
    use hawier_api::ext::ConsoleSurface;

    /// Every declared view names a resource and carries a query or a sort.
    ///
    /// These used to walk `declaration().surfaces`. With no surface left they
    /// would have passed over an empty vector forever — a green test that
    /// checks nothing is worse than no test, so they walk the views instead.
    #[test]
    fn every_contribution_names_a_resource_and_does_something() {
        let declaration = declaration();
        assert!(
            !declaration.views.is_empty(),
            "this module exists to contribute something"
        );
        for contribution in declaration.views {
            assert!(
                !contribution.resource.trim().is_empty(),
                "a view has to be attached to a resource"
            );
            assert!(!contribution.views.is_empty(), "{}", contribution.resource);
            for view in contribution.views {
                assert!(
                    view.q.is_some() || view.sort.is_some(),
                    "{}/{} filters nothing and sorts nothing, so it is the \
                     resource's default list under a second name",
                    contribution.resource,
                    view.id
                );
            }
        }
    }

    #[test]
    fn view_ids_are_unique_within_a_resource() {
        for contribution in declaration().views {
            let mut ids: Vec<&str> = contribution.views.iter().map(|v| v.id.as_str()).collect();
            let before = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(
                before,
                ids.len(),
                "duplicate view id in {}",
                contribution.resource
            );
        }
    }

    /// A surface still round-trips through the ABI even though nothing here
    /// declares one — the declaration type is the console's contract for
    /// future use, and it must not rot while unused.
    #[test]
    fn the_surface_shape_is_still_intact() {
        let surface = ConsoleSurface {
            id: "example".to_string(),
            label: "Example".to_string(),
            endpoint: "targets".to_string(),
            group: "inventory".to_string(),
            id_field: "controllerId".to_string(),
            columns: vec!["controllerId".to_string()],
            default_sort: None,
            search_fields: vec!["controllerId".to_string()],
            icon: Some("health".to_string()),
            views: vec![view("all", "All", None, Some("controllerId:ASC"))],
        };
        let encoded = serde_json::to_value(ConsoleDeclaration {
            surfaces: vec![surface],
            views: Vec::new(),
        })
        .expect("a declaration must encode");
        let back: ConsoleDeclaration =
            serde_json::from_value(encoded).expect("and decode to the same thing");
        assert_eq!(back.surfaces.len(), 1);
        assert_eq!(back.surfaces[0].icon.as_deref(), Some("health"));
    }
}
