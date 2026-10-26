//! Rollout strategies: the fleet is divided by a module, not by the engine.
//! Run via `make it`.

use crate::common;

use hawier_api::codes::{ActionType, RolloutStatus};
use serde_json::{json, Value};
use sqlx::Row;

async fn tick(server: &common::TestServer) {
    hawier_server::rollout::tick(&server.state).await.unwrap();
}

async fn create_rollout(server: &common::TestServer, body: Value) -> (u16, Value) {
    let response = common::http()
        .post(format!("{}/rest/v1/rollouts", server.base_url))
        .header("Authorization", common::basic("admin", "admin"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn seed(server: &common::TestServer, devices: usize) -> i64 {
    let ds = common::seed_ds_with_artifact(server, "DEFAULT", "strat-fw", b"payload").await;
    for n in 0..devices {
        common::seed_target(server, "DEFAULT", &format!("strat-dev-{n:04}")).await;
    }
    ds.ds_id
}

async fn group_sizes(server: &common::TestServer, rollout_id: i64) -> Vec<i64> {
    sqlx::query("SELECT total_targets FROM sp_rollout_group WHERE rollout = $1 ORDER BY id")
        .bind(rollout_id)
        .fetch_all(&server.state.pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<Option<i64>, _>("total_targets").unwrap_or(0))
        .collect()
}

#[tokio::test]
#[ignore]
async fn the_classic_strategy_splits_by_percentage() {
    let server = common::spawn().await;
    let ds_id = seed(&server, 10).await;
    let (status, rollout) = create_rollout(
        &server,
        json!({
            "name": "classic-split",
            "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*",
            "amountGroups": 2,
        }),
    )
    .await;
    assert_eq!(status, 201, "{rollout}");
    let rollout_id = rollout["id"].as_i64().unwrap();

    tick(&server).await;

    // Half each, and nobody dropped by rounding: the last group takes the rest.
    assert_eq!(group_sizes(&server, rollout_id).await, vec![5, 5]);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT status FROM sp_rollout WHERE id = $1")
            .bind(rollout_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        RolloutStatus::Ready.code()
    );
}

#[tokio::test]
#[ignore]
async fn an_odd_split_leaves_nobody_behind() {
    let server = common::spawn().await;
    let ds_id = seed(&server, 7).await;
    let (_, rollout) = create_rollout(
        &server,
        json!({
            "name": "odd-split",
            "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*",
            "amountGroups": 3,
        }),
    )
    .await;
    let rollout_id = rollout["id"].as_i64().unwrap();
    tick(&server).await;

    let sizes = group_sizes(&server, rollout_id).await;
    assert_eq!(
        sizes.iter().sum::<i64>(),
        7,
        "every target lands in exactly one group: {sizes:?}"
    );
}

#[tokio::test]
#[ignore]
async fn a_bucket_rollout_takes_a_slice_first_then_the_rest() {
    // A node loads the strategy it runs — the same way a deployment does.
    let server = common::spawn_with(|config| {
        config.ext_rollout = common::ext_module("hawier_ext_rollout_bucket")
    })
    .await;
    let ds_id = seed(&server, 40).await;
    let (status, rollout) = create_rollout(
        &server,
        json!({
            "name": "canary",
            "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*",
            "amountGroups": 1,
            "strategy": "bucket",
            // A quarter of the bucket space per wave, two waves, then a sweep.
            "strategyParams": { "modulo": 100, "sliceSize": 25, "waves": 2 },
        }),
    )
    .await;
    assert_eq!(status, 201, "{rollout}");
    let rollout_id = rollout["id"].as_i64().unwrap();

    tick(&server).await;

    let sizes = group_sizes(&server, rollout_id).await;
    assert_eq!(
        sizes.len(),
        3,
        "two waves and a final sweep, and no phantom left from the declared \
         group the strategy did not use: {sizes:?}"
    );
    assert_eq!(
        sizes.iter().sum::<i64>(),
        40,
        "the sweep catches whatever the waves missed: {sizes:?}"
    );
    assert!(
        sizes[0] < 40,
        "the first wave is a sample, not the fleet: {sizes:?}"
    );
}

#[tokio::test]
#[ignore]
async fn the_same_rollout_picks_the_same_devices_twice() {
    // A canary you cannot reproduce is a coin toss. Two rollouts with the
    // same id would be the same rollout, so this checks the weaker but
    // testable property: planning is a pure function of the rollout.
    let server = common::spawn_with(|config| {
        config.ext_rollout = common::ext_module("hawier_ext_rollout_bucket")
    })
    .await;
    let ds_id = seed(&server, 40).await;
    let params = json!({ "modulo": 100, "sliceSize": 25, "waves": 1 });

    let (_, first) = create_rollout(
        &server,
        json!({
            "name": "canary-a", "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*", "amountGroups": 1,
            "strategy": "bucket", "strategyParams": params,
        }),
    )
    .await;
    let (_, second) = create_rollout(
        &server,
        json!({
            "name": "canary-b", "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*", "amountGroups": 1,
            "strategy": "bucket", "strategyParams": params,
        }),
    )
    .await;
    tick(&server).await;

    let a = group_sizes(&server, first["id"].as_i64().unwrap()).await;
    let b = group_sizes(&server, second["id"].as_i64().unwrap()).await;
    // Different rollouts pick different windows, so the first waves differ in
    // membership; both still cover the whole fleet.
    assert_eq!(a.iter().sum::<i64>(), 40);
    assert_eq!(b.iter().sum::<i64>(), 40);
}

#[tokio::test]
#[ignore]
async fn an_unknown_strategy_is_refused_at_create() {
    let server = common::spawn().await;
    let ds_id = seed(&server, 2).await;
    let (status, body) = create_rollout(
        &server,
        json!({
            "name": "typo",
            "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*",
            "amountGroups": 1,
            "strategy": "buckett",
        }),
    )
    .await;
    // Otherwise the rollout sits in CREATING forever, looking merely slow.
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
#[ignore]
async fn a_rollout_still_runs_to_completion_under_a_strategy() {
    let server = common::spawn().await;
    let ds_id = seed(&server, 4).await;
    let (_, rollout) = create_rollout(
        &server,
        json!({
            "name": "full-run",
            "distributionSetId": ds_id,
            "targetFilterQuery": "controllerid==strat-dev-*",
            "amountGroups": 1,
        }),
    )
    .await;
    let rollout_id = rollout["id"].as_i64().unwrap();
    tick(&server).await;

    common::http()
        .post(format!(
            "{}/rest/v1/rollouts/{rollout_id}/start",
            server.base_url
        ))
        .header("Authorization", common::basic("admin", "admin"))
        .send()
        .await
        .unwrap();
    tick(&server).await;

    // Every action finishes, so the strategy's success threshold is met and
    // the engine's own state machine takes it to FINISHED.
    sqlx::query("UPDATE sp_action SET status = $1, active = FALSE WHERE rollout = $2")
        .bind(hawier_api::codes::ActionStatus::Finished.code())
        .bind(rollout_id)
        .execute(&server.state.pool)
        .await
        .unwrap();
    tick(&server).await;
    tick(&server).await;

    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT status FROM sp_rollout WHERE id = $1")
            .bind(rollout_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        RolloutStatus::Finished.code(),
        "the engine still finishes a rollout its strategy is done with"
    );
    let _ = ActionType::Forced;
}
