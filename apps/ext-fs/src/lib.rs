//! hawier storage extension: a directory on this machine.
//!
//! Answers "where is artifact X" with a path, and nothing else. It never
//! opens the file — hawier does that, because bytes do not cross the
//! extension boundary. What lives here is the one genuine decision: the
//! layout of the tree.
//!
//! Runs as a wasm module with no filesystem and no network of its own: it
//! only ever builds a string. Configure with `HAWIER_EXT_FS_ROOT`.

use hawier_api::ext::{
    points, ExecutionSite, ExtManifest, ExtRequest, ExtResponse, StorageLocate, StorageLocation,
};

fn manifest() -> ExtManifest {
    ExtManifest {
        name: "hawier-ext-fs".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![points::STORAGE_LOCATE.to_string()],
        // Artifacts are served by every node, control or sub.
        sites: vec![ExecutionSite::Ddi, ExecutionSite::Control],
        // None: it answers with a path and touches nothing.
        capabilities: Vec::new(),
        deadline_ms: 1_000,
    }
}

fn call(request: ExtRequest) -> ExtResponse {
    if request.point != points::STORAGE_LOCATE {
        return ExtResponse::Err {
            message: format!("hawier-ext-fs does not implement {}", request.point),
        };
    }
    match serde_json::from_value::<StorageLocate>(request.payload) {
        Ok(locate) => match serde_json::to_value(locate_artifact(&locate)) {
            Ok(payload) => ExtResponse::Ok { payload },
            Err(error) => ExtResponse::Err {
                message: format!("cannot encode a location: {error}"),
            },
        },
        Err(error) => ExtResponse::Err {
            message: format!("unusable storage.locate payload: {error}"),
        },
    }
}

/// hawkBit's `FileArtifactStorage` layout, so a stock hawkBit can share the
/// same directory during the dev stage:
/// `{root}/{TENANT}/{sha1[-4..-2]}/{sha1[-2..]}/{sha1}`.
///
/// The path is not canonicalised here on purpose: hawier refuses any path
/// outside its own artifact root anyway, and a descriptor is data, not
/// authority. That also keeps this module free of filesystem access.
fn locate_artifact(locate: &StorageLocate) -> StorageLocation {
    let root =
        std::env::var("HAWIER_EXT_FS_ROOT").unwrap_or_else(|_| "/var/lib/hawier".to_string());
    let sha1 = &locate.sha1;
    if sha1.len() < 4 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        // Not a name we will build a path from.
        return StorageLocation::Absent;
    }
    StorageLocation::Path {
        path: format!(
            "{}/{}/{}/{}/{sha1}",
            root.trim_end_matches('/'),
            locate.tenant.to_uppercase(),
            &sha1[sha1.len() - 4..sha1.len() - 2],
            &sha1[sha1.len() - 2..]
        ),
    }
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod tests {
    use super::*;
    use hawier_api::ext::StorageOp;

    fn locate(sha1: &str) -> StorageLocation {
        locate_artifact(&StorageLocate {
            tenant: "default".to_string(),
            sha1: sha1.to_string(),
            op: StorageOp::Get,
        })
    }

    #[test]
    fn fans_out_on_the_last_four_hex_digits() {
        std::env::set_var("HAWIER_EXT_FS_ROOT", "/srv/artifacts");
        let StorageLocation::Path { path } = locate("2aae6c35c94fcfb415dbe95f408b9ce91ee846ed")
        else {
            panic!("expected a path");
        };
        assert_eq!(
            path,
            "/srv/artifacts/DEFAULT/46/ed/2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
        );
    }

    #[test]
    fn a_traversing_sha1_is_absent_not_a_path() {
        assert!(matches!(
            locate("../../etc/passwd"),
            StorageLocation::Absent
        ));
        assert!(matches!(locate("zz"), StorageLocation::Absent));
    }
}
