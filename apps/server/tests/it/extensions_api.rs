//! `/rest/v1/system/extensions` — the inventory and the parameters file.
//!
//! Run via `make it`.

use crate::common;

use serde_json::{json, Value};

fn temp_file(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("hawier-it-ext-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join(format!("{name}.env"))
}

async fn call(
    server: &common::TestServer,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut request = common::http()
        .request(method, format!("{}{path}", server.base_url))
        .header("Authorization", common::basic("admin", "admin"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn inventory(server: &common::TestServer) -> Value {
    let (status, body) = call(
        server,
        reqwest::Method::GET,
        "/rest/v1/system/extensions",
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body
}

fn parameter<'a>(inventory: &'a Value, name: &str) -> Option<&'a Value> {
    inventory["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["name"] == name)
}

/// Every point is listed, configured or not, and an absent one says what its
/// absence costs.
///
/// A row that simply vanished when unconfigured would leave an operator
/// unable to tell "this node cannot do that" from "I forgot it exists".
#[tokio::test]
#[ignore]
async fn the_inventory_names_every_point_even_the_unconfigured_ones() {
    let server = common::spawn_with(|config| {
        // Storage only: the rest must still appear.
        config.ext_event_sink = String::new();
        config.ext_retention = String::new();
        config.ext_rollout = String::new();
        config.ext_console = String::new();
        config.ext_authz = String::new();
    })
    .await;

    let body = inventory(&server).await;
    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 6, "one row per slot: {body}");
    for point in points {
        assert!(!point["absence"].as_str().unwrap().is_empty(), "{point}");
        assert!(!point["envVar"].as_str().unwrap().is_empty(), "{point}");
        assert!(!point["points"].as_array().unwrap().is_empty(), "{point}");
    }

    let storage = points.iter().find(|p| p["key"] == "storage").unwrap();
    assert_eq!(storage["configured"], true);
    assert_eq!(storage["module"]["name"], "hawier-ext-fs");
    assert!(storage["module"]["path"]
        .as_str()
        .unwrap()
        .ends_with("hawier_ext_fs.wasm"));

    let authz = points.iter().find(|p| p["key"] == "authz").unwrap();
    assert_eq!(authz["configured"], false);
    assert_eq!(authz["module"], Value::Null);

    // The rollout slot serves two points on one module: an inventory that
    // listed points rather than slots would show the module twice.
    let rollout = points.iter().find(|p| p["key"] == "rollout").unwrap();
    assert_eq!(rollout["points"].as_array().unwrap().len(), 2);
}

/// The deadline a module declares and the deadline the node enforces are two
/// different facts, and the node has only ever used its own.
#[tokio::test]
#[ignore]
async fn the_declared_deadline_is_reported_next_to_the_enforced_one() {
    let server = common::spawn_with(|config| config.ext_deadline_ms = 2_000).await;
    let body = inventory(&server).await;
    assert_eq!(body["node"]["enforcedDeadlineMs"], 2_000);

    let storage = body["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "storage")
        .unwrap();
    // hawier-ext-fs declares 1000; the host still applies 2000.
    assert_eq!(storage["module"]["declaredDeadlineMs"], 1_000);
}

/// A parameter written through the API reaches the file, and does not reach
/// the running node — which is the whole contract: extensions are not
/// restartable, nodes are.
#[tokio::test]
#[ignore]
async fn a_parameter_written_through_the_api_reaches_the_file_but_not_the_running_node() {
    let path = temp_file("written");
    std::fs::write(&path, "# seeded\nHAWIER_EXT_S3_BUCKET=old\n").unwrap();
    let server =
        common::spawn_with(|config| config.ext_params_file = path.display().to_string()).await;

    let before = inventory(&server).await;
    assert_eq!(before["drift"]["changed"], 0);
    assert_eq!(
        parameter(&before, "HAWIER_EXT_S3_BUCKET").unwrap()["value"],
        "old"
    );

    let (status, _) = call(
        &server,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_BUCKET",
        Some(json!({ "value": "new" })),
    )
    .await;
    assert_eq!(status, 200);

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("new"), "the file is what was written: {text}");
    assert!(text.contains("# seeded"), "a comment was eaten: {text}");

    let after = inventory(&server).await;
    assert_eq!(
        after["drift"]["changed"], 1,
        "the node must say it is running on something else now"
    );
    assert_eq!(
        parameter(&after, "HAWIER_EXT_S3_BUCKET").unwrap()["changedSinceStart"],
        true
    );
    // And the module still has the old value: nothing was hot-reloaded.
    assert_eq!(
        server
            .state
            .ext
            .params
            .get("HAWIER_EXT_S3_BUCKET")
            .unwrap()
            .value,
        "old"
    );
    std::fs::remove_file(&path).ok();
}

/// The environment pins a value against the file, loudly.
#[tokio::test]
#[ignore]
async fn the_environment_pins_a_parameter_against_the_file() {
    let path = temp_file("pinned");
    std::fs::write(&path, "HAWIER_EXT_FS_ROOT=/from-file\n").unwrap();
    // The harness sets this in the process environment already.
    let server =
        common::spawn_with(|config| config.ext_params_file = path.display().to_string()).await;

    let body = inventory(&server).await;
    let pinned = parameter(&body, "HAWIER_EXT_FS_ROOT").unwrap();
    assert_eq!(pinned["origin"], "environment");
    assert_eq!(pinned["inFile"], true, "the shadowed line stays visible");
    assert_eq!(pinned["editable"], false, "no form for an inert edit");

    let (status, error) = call(
        &server,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_FS_ROOT",
        Some(json!({ "value": "/elsewhere" })),
    )
    .await;
    assert_eq!(status, 409, "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("pinned"),
        "the refusal must say why: {error}"
    );
    std::fs::remove_file(&path).ok();
}

/// A name that selects which module the node loads cannot be set here.
///
/// This is the trust boundary, not tidiness: the host grants a module's
/// http_out allowlist from the module's own manifest, so choosing the module
/// is choosing what the node may reach.
#[tokio::test]
#[ignore]
async fn a_reserved_name_cannot_be_set_through_the_api() {
    let path = temp_file("reserved");
    std::fs::write(&path, "").unwrap();
    let server =
        common::spawn_with(|config| config.ext_params_file = path.display().to_string()).await;

    for name in [
        "HAWIER_EXT_STORAGE",
        "HAWIER_EXT_AUTHZ",
        "HAWIER_EXT_DEADLINE_MS",
        "HAWIER_EXT_PARAMS_FILE",
    ] {
        let (status, error) = call(
            &server,
            reqwest::Method::PUT,
            &format!("/rest/v1/system/extensions/parameters/{name}"),
            Some(json!({ "value": "/tmp/anything.wasm" })),
        )
        .await;
        assert_eq!(status, 400, "{name} was accepted: {error}");
    }
    // And a name that could never reach a module at all.
    let (status, _) = call(
        &server,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/PATH",
        Some(json!({ "value": "/usr/bin" })),
    )
    .await;
    assert_eq!(status, 400);

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    std::fs::remove_file(&path).ok();
}

/// A secret is settable and never readable.
#[tokio::test]
#[ignore]
async fn a_secret_parameter_never_comes_back_out() {
    let path = temp_file("secret");
    std::fs::write(&path, "").unwrap();
    let server =
        common::spawn_with(|config| config.ext_params_file = path.display().to_string()).await;

    let (status, _) = call(
        &server,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_SECRET_KEY",
        Some(json!({ "value": "sup3r-s3cret-value" })),
    )
    .await;
    assert_eq!(status, 200);

    // It really was written — otherwise the rest of this passes vacuously.
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("sup3r-s3cret-value"), "{text}");

    let body = inventory(&server).await;
    assert!(
        !body.to_string().contains("sup3r-s3cret-value"),
        "the inventory handed a credential back: {body}"
    );
    let secret = parameter(&body, "HAWIER_EXT_S3_SECRET_KEY").unwrap();
    assert_eq!(secret["secret"], true);
    assert_eq!(secret["value"], Value::Null);
    assert_eq!(secret["set"], true, "but the operator can see it is set");

    // An empty save keeps it: an HTML input always submits something, and
    // clear-on-empty would wipe a credential on any accidental save.
    let (status, _) = call(
        &server,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_SECRET_KEY",
        Some(json!({ "value": "" })),
    )
    .await;
    assert_eq!(status, 200);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("sup3r-s3cret-value"));

    // Clearing is the explicit delete.
    let (status, _) = call(
        &server,
        reqwest::Method::DELETE,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_SECRET_KEY",
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("sup3r-s3cret-value"));
    std::fs::remove_file(&path).ok();
}

/// A sub node reports its own configuration and refuses to be configured.
#[tokio::test]
#[ignore]
async fn a_sub_node_reports_but_refuses_to_write() {
    let path = temp_file("sub");
    std::fs::write(&path, "HAWIER_EXT_S3_BUCKET=shared\n").unwrap();
    let control = common::spawn().await;
    let sub = common::spawn_on(&control, |config| {
        config.role = hawier_server::config::Role::Sub;
        config.ext_params_file = path.display().to_string();
    })
    .await;

    let body = inventory(&sub).await;
    assert_eq!(body["node"]["role"], "sub");
    assert_eq!(
        body["node"]["writable"], false,
        "a sub node says parameters are set elsewhere"
    );
    assert_eq!(
        parameter(&body, "HAWIER_EXT_S3_BUCKET").unwrap()["value"],
        "shared",
        "but it reports its own file truthfully"
    );

    let (status, _) = call(
        &sub,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_BUCKET",
        Some(json!({ "value": "no" })),
    )
    .await;
    assert_eq!(status, 405, "writes are refused on a sub node");
    std::fs::remove_file(&path).ok();
}

/// A file that cannot be written does not take the inventory down with it.
#[tokio::test]
#[ignore]
async fn an_unwritable_file_does_not_take_the_endpoint_down() {
    let server = common::spawn_with(|config| {
        config.ext_params_file = "/nonexistent-directory/hawier.env".to_string();
    })
    .await;

    let body = inventory(&server).await;
    assert_eq!(body["node"]["paramsFileState"], "absent");
    assert_eq!(body["node"]["paramsFileWritable"], false);

    let (status, _) = call(
        &server,
        reqwest::Method::PUT,
        "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_BUCKET",
        Some(json!({ "value": "x" })),
    )
    .await;
    assert_eq!(status, 500, "permanent, so not a 503 with Retry-After");
}

/// A directory this process cannot write into is reported as unwritable.
///
/// The first version asked `permissions().readonly()`, which reports the mode
/// and says nothing about whether *this* process may write. The shipped
/// container mounts the volume as root and runs the node as `hawier`: the
/// mode said "writable", the console offered Save buttons, and every one of
/// them would have answered 500.
#[tokio::test]
#[ignore]
async fn a_directory_this_process_cannot_write_is_not_writable() {
    use std::os::unix::fs::PermissionsExt;

    let directory = std::env::temp_dir().join(format!("hawier-it-ro-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("extensions.env");
    // r-x for everyone: the mode is not "readonly" in Rust's sense, and yet
    // nothing may be created here.
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555)).unwrap();

    let server =
        common::spawn_with(|config| config.ext_params_file = path.display().to_string()).await;
    let body = inventory(&server).await;
    assert_eq!(
        body["node"]["paramsFileWritable"], false,
        "the page must not offer a save that cannot happen"
    );

    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(&directory).ok();
}

/// Reading the inventory needs READ_SYSTEM; writing needs UPDATE_SYSTEM.
#[tokio::test]
#[ignore]
async fn reading_the_inventory_needs_read_system() {
    let path = temp_file("authz");
    std::fs::write(&path, "HAWIER_EXT_S3_BUCKET=b\n").unwrap();
    let server =
        common::spawn_with(|config| config.ext_params_file = path.display().to_string()).await;

    // Granted to `limited`, not to admin: the authorisation cache is
    // process-wide and keyed by (tenant, user, permission), so giving admin a
    // restricted role here would decide what every concurrent test in this
    // binary sees for admin. An administrator has to exist too, or the write
    // creating the first grant would leave the tenant without one.
    let admin_role: i64 = server.state.incidents.next();
    let reader_role: i64 = server.state.incidents.next();
    for (id, name, permissions) in [
        (admin_role, "administrator", "ROLE_ALL"),
        (reader_role, "reader", "READ_SYSTEM"),
    ] {
        sqlx::query(
            "INSERT INTO hawier_role (id, tenant, name, permissions) VALUES ($1, 'DEFAULT', $2, $3)",
        )
        .bind(id)
        .bind(name)
        .bind(permissions)
        .execute(&server.state.pool)
        .await
        .unwrap();
    }
    for (username, role) in [("admin", admin_role), ("limited", reader_role)] {
        sqlx::query(
            "INSERT INTO hawier_grant (id, tenant, username, role) VALUES ($1, 'DEFAULT', $2, $3)",
        )
        .bind(server.state.incidents.next())
        .bind(username)
        .bind(role)
        .execute(&server.state.pool)
        .await
        .unwrap();
    }
    server.state.authz_cache.invalidate();

    let limited = |method: reqwest::Method, path: String, body: Option<Value>| {
        let url = format!("{}{path}", server.base_url);
        async move {
            let mut request = common::http()
                .request(method, url)
                .header("Authorization", common::basic("limited", "limited"));
            if let Some(body) = body {
                request = request.json(&body);
            }
            request.send().await.unwrap().status().as_u16()
        }
    };

    assert_eq!(
        limited(
            reqwest::Method::GET,
            "/rest/v1/system/extensions".to_string(),
            None
        )
        .await,
        200,
        "READ_SYSTEM is enough to look"
    );
    assert_eq!(
        limited(
            reqwest::Method::PUT,
            "/rest/v1/system/extensions/parameters/HAWIER_EXT_S3_BUCKET".to_string(),
            Some(json!({ "value": "c" })),
        )
        .await,
        403,
        "but not to change"
    );
    std::fs::remove_file(&path).ok();
}

/// A file that is there and is not a parameters file stops the node.
#[tokio::test]
#[ignore]
async fn a_malformed_parameters_file_stops_the_node_from_starting() {
    let path = temp_file("malformed");
    // Two values for one credential: booting on either silently is worse
    // than not booting.
    std::fs::write(&path, "HAWIER_EXT_S3_BUCKET=a\nHAWIER_EXT_S3_BUCKET=b\n").unwrap();

    let mut config = common::test_config("postgres://hawkbit:hawkbit@localhost:5432/hawkbit");
    config.ext_params_file = path.display().to_string();
    let message = match hawier_server::init(config).await {
        Ok(_) => panic!("a duplicate name must stop the node, and did not"),
        Err(error) => format!("{error:?}"),
    };
    assert!(message.contains("HAWIER_EXT_S3_BUCKET"), "{message}");
    std::fs::remove_file(&path).ok();
}
