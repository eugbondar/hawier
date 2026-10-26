//! What a node does when an extension is missing or broken. Run via `make it`.
//!
//! There is no builtin fallback by design: a decision that belongs to an
//! extension is either taken by that extension or not taken at all. For
//! artifacts that means `503` with `Retry-After` — busy, not broken — so a
//! fleet backs off instead of hammering a node that cannot serve it.

use crate::common;

use hawier_api::codes::ActionType;
use hawier_api::ext::{points, ExtRequest, StorageLocate, StorageOp};

/// Fetch an artifact over DDI the way a device would.
async fn download(server: &common::TestServer, target: &common::SeededTarget, sm_id: i64) -> u16 {
    common::http()
        .get(format!(
            "{}/DEFAULT/controller/v1/{}/softwaremodules/{}/artifacts/ext-fw.bin",
            server.base_url, target.controller_id, sm_id
        ))
        .header("Authorization", format!("TargetToken {}", target.token))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
#[ignore]
async fn with_a_storage_extension_a_download_works() {
    let server = common::spawn().await;
    let ds = common::seed_ds_with_artifact(&server, "DEFAULT", "ext-fw", b"payload").await;
    let target = common::seed_target(&server, "DEFAULT", "ext-dev-1").await;
    common::assign_ds(&server, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    assert_eq!(download(&server, &target, ds.sm_id).await, 200);
}

#[tokio::test]
#[ignore]
async fn without_a_storage_extension_artifacts_answer_503() {
    // Seed the row through a node that has an extension, then serve it from
    // a node that has none: the artifact exists, this node simply cannot
    // reach it. That is exactly the sub-node-with-a-dead-sidecar case.
    let seeded = common::spawn().await;
    let ds = common::seed_ds_with_artifact(&seeded, "DEFAULT", "ext-fw", b"payload").await;
    let target = common::seed_target(&seeded, "DEFAULT", "ext-dev-2").await;
    common::assign_ds(&seeded, "DEFAULT", target.id, ds.ds_id, ActionType::Forced).await;

    let blind = common::spawn_on(&seeded, |config| config.ext_storage = String::new()).await;
    let response = common::http()
        .get(format!(
            "{}/DEFAULT/controller/v1/{}/softwaremodules/{}/artifacts/ext-fw.bin",
            blind.base_url, target.controller_id, ds.sm_id
        ))
        .header("Authorization", format!("TargetToken {}", target.token))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status().as_u16(),
        503,
        "no storage extension must be 503, never 500 or a silent fallback"
    );
    assert!(
        response.headers().contains_key("retry-after"),
        "a device needs to be told when to come back"
    );
}

/// A module that is not there stops the node from starting.
///
/// Stricter than the runtime failure below, and deliberately so: the path is
/// local configuration, so a typo is the operator's mistake and should be
/// loud at boot rather than a node that runs and answers 503 forever.
#[tokio::test]
#[ignore]
async fn a_missing_module_stops_the_node_from_starting() {
    let db_url = common::create_database().await;
    let mut config = common::test_config(&db_url);
    config.ext_storage = "/nonexistent/hawier_ext_fs.wasm".to_string();

    let Err(error) = hawier_server::init(config).await else {
        panic!("a node with an unloadable extension must not start");
    };
    let text = format!("{error:#}");
    assert!(
        text.contains("wasm extension"),
        "the failure must name the extension: {text}"
    );
}

/// The extension itself, driven straight from the test.
///
/// Loading the module here rather than through the server keeps the wire
/// format honest independently of the host, and proves the sandbox: this
/// instance gets a clock and `HAWIER_EXT_*` and nothing else.
#[tokio::test]
#[ignore]
async fn the_fs_extension_answers_on_its_own() {
    let extension = hawier_server::ext::wasm::Extension::new(
        points::STORAGE_LOCATE,
        &common::ext_module("hawier_ext_fs"),
        std::time::Duration::from_secs(5),
        Vec::new(),
    )
    .expect("load hawier_ext_fs");

    let payload = extension
        .call(ExtRequest {
            point: points::STORAGE_LOCATE.to_string(),
            payload: serde_json::to_value(StorageLocate {
                tenant: "default".to_string(),
                sha1: "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed".to_string(),
                op: StorageOp::Get,
            })
            .unwrap(),
        })
        .await
        .expect("locate");

    assert_eq!(payload["kind"], "path");
    let path = payload["path"].as_str().unwrap();
    assert!(
        path.ends_with("DEFAULT/46/ed/2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"),
        "unexpected layout: {path}"
    );

    // The handshake is what proved it implements this point at all.
    let manifest = extension.manifest();
    assert_eq!(manifest.name, "hawier-ext-fs");
    assert!(manifest.capabilities.is_empty(), "it asks for no authority");
}

/// The manifest is known before anything has been asked of the module.
///
/// It used to be read off whichever instance happened to be idle, and the
/// pool starts empty — so on a node with no rollouts and no retention rules,
/// those slots would have reported "unknown" for the life of the process.
/// An inventory that cannot say what is loaded is not an inventory.
#[tokio::test]
#[ignore]
async fn a_manifest_is_available_before_the_first_call() {
    let extension = hawier_server::ext::wasm::Extension::new(
        points::STORAGE_LOCATE,
        &common::ext_module("hawier_ext_fs"),
        std::time::Duration::from_secs(5),
        Vec::new(),
    )
    .expect("load hawier_ext_fs");

    // No call has been made.
    let manifest = extension.manifest();
    assert_eq!(manifest.name, "hawier-ext-fs");
    assert!(manifest.points.iter().any(|p| p == points::STORAGE_LOCATE));
    assert!(
        manifest.deadline_ms > 0,
        "a module declares its own deadline even though the host enforces its own"
    );
    // And the two are different facts, deliberately: the host's deadline is
    // the node's setting, the module's is a hint it has never been given.
    assert_eq!(extension.deadline(), std::time::Duration::from_secs(5));
    assert_eq!(extension.point(), points::STORAGE_LOCATE);
    assert!(extension.path().ends_with("hawier_ext_fs.wasm"));
}

/// A module that does not implement the point must fail at load, not on the
/// first real call.
///
/// This used to be aspirational: the handshake happened when the pool was
/// first filled, which is on the first call, so a node configured with the
/// wrong module started happily and broke at whatever hour the first
/// artifact download arrived. Now the module is instantiated at load, so it
/// breaks where a configuration mistake belongs — at startup.
#[tokio::test]
#[ignore]
async fn a_module_for_another_point_is_refused() {
    let loaded = hawier_server::ext::wasm::Extension::new(
        points::STORAGE_LOCATE,
        &common::ext_module("hawier_ext_webhook"),
        std::time::Duration::from_secs(5),
        Vec::new(),
    );
    let message = match loaded {
        Ok(_) => panic!("the webhook sink is not a storage backend, yet it loaded"),
        Err(error) => format!("{error:?}"),
    };
    assert!(
        message.contains("hawier-ext-webhook") && message.contains(points::STORAGE_LOCATE),
        "the refusal must name the module and the point it was asked for: {message}"
    );
}

// ---------------------------------------------------------------------------
// The http_out capability
// ---------------------------------------------------------------------------

/// A stand-in signing service, so the test does not need the internet.
async fn spawn_signer() -> (String, std::sync::Arc<std::sync::Mutex<usize>>) {
    use axum::extract::State;
    use axum::routing::post;
    use axum::{Json, Router};

    let calls = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let app = Router::new()
        .route(
            "/sign",
            post(
                |State(calls): State<std::sync::Arc<std::sync::Mutex<usize>>>,
                 Json(body): Json<serde_json::Value>| async move {
                    *calls.lock().unwrap() += 1;
                    Json(serde_json::json!({
                        "url": format!(
                            "https://signed.invalid/{}?token=from-service",
                            body["key"].as_str().unwrap_or_default()
                        )
                    }))
                },
            ),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (format!("http://{addr}/sign"), calls)
}

/// Multi-threaded on purpose: this test hosts the service it calls, and a
/// guest call blocks the thread it runs on until it answers. With a single
/// thread the test would be waiting on a server it is itself preventing from
/// running. The node is multi-threaded for the same reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn an_extension_can_have_the_host_make_a_request_for_it() {
    let (signer, calls) = spawn_signer().await;
    // The module declares the capability only when this is set, and the host
    // grants exactly what the manifest names.
    // This machine routes egress through a proxy, and a service on loopback
    // is not reachable that way. Deployments do the same for the same reason
    // (deploy/docker-compose.yml clears the proxy for hawier services).
    std::env::set_var("no_proxy", "127.0.0.1,localhost");
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");

    // The same resolution the node performs, over an environment handed in
    // rather than exported. `HAWIER_EXT_*` is read once per node and beats
    // the parameters file, so exporting these would decide what every later
    // test in this process resolves — and the suite is one process.
    let params = hawier_server::ext::params::Resolved::from_parts(
        None,
        [
            ("HAWIER_EXT_S3_SIGNER_URL", signer.as_str()),
            ("HAWIER_EXT_S3_BUCKET", "artifacts"),
            ("HAWIER_EXT_S3_ENDPOINT", "https://s3.invalid"),
            ("HAWIER_EXT_S3_ACCESS_KEY", "key"),
            ("HAWIER_EXT_S3_SECRET_KEY", "secret"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    );
    let extension = hawier_server::ext::wasm::Extension::new(
        points::STORAGE_LOCATE,
        &common::ext_module("hawier_ext_s3"),
        std::time::Duration::from_secs(10),
        params.for_guest(),
    )
    .expect("load hawier_ext_s3");

    let payload = extension
        .call(ExtRequest {
            point: points::STORAGE_LOCATE.to_string(),
            payload: serde_json::to_value(StorageLocate {
                tenant: "DEFAULT".to_string(),
                sha1: "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed".to_string(),
                op: StorageOp::Get,
            })
            .unwrap(),
        })
        .await
        .expect("locate");

    assert_eq!(*calls.lock().unwrap(), 1, "the signing service was asked");
    assert_eq!(payload["kind"], "url");
    assert!(
        payload["url"]
            .as_str()
            .unwrap_or_default()
            .contains("from-service"),
        "the link came from the service, not from local signing: {payload}"
    );
}

#[tokio::test]
#[ignore]
async fn a_module_that_did_not_declare_http_cannot_reach_anything() {
    let (signer, calls) = spawn_signer().await;
    // The fs module never asks for http_out. Even pointed at a live service
    // it must not be able to call it — the host grants what the manifest
    // declares, not what the code attempts.
    let params = hawier_server::ext::params::Resolved::from_parts(
        None,
        [("HAWIER_EXT_FS_ROOT".to_string(), "/tmp".to_string())]
            .into_iter()
            .collect(),
    );
    let extension = hawier_server::ext::wasm::Extension::new(
        points::STORAGE_LOCATE,
        &common::ext_module("hawier_ext_fs"),
        std::time::Duration::from_secs(10),
        params.for_guest(),
    )
    .expect("load hawier_ext_fs");

    extension
        .call(ExtRequest {
            point: points::STORAGE_LOCATE.to_string(),
            payload: serde_json::to_value(StorageLocate {
                tenant: "DEFAULT".to_string(),
                sha1: "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed".to_string(),
                op: StorageOp::Get,
            })
            .unwrap(),
        })
        .await
        .expect("locate");

    let manifest = extension.manifest();
    assert!(
        manifest.capabilities.is_empty(),
        "hawier-ext-fs asks for no authority at all"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        0,
        "and reached nothing, whatever the environment said: {signer}"
    );
}
