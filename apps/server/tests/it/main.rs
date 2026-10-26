//! The node's integration suite: one binary, one module per area.
//!
//! Cargo mints an executable for every file directly under `tests/`, and
//! fifteen of them linking wasmtime cost 6.5 GB of `target/` per build
//! configuration — for fifteen copies of the same dependency closure. They
//! are modules here instead, which also means editing the harness relinks one
//! binary rather than fifteen.
//!
//! Everything here is `#[ignore]`d and wants a real Postgres: `make it`.
//! A single area is `cargo test -p hawier-server --test it -- --ignored ddi::`.

mod common;

mod authz;
mod correctness;
mod ddi;
mod errors;
mod events;
mod ext;
mod extensions_api;
mod maintenance;
mod mgmt;
mod phase1;
mod retention;
mod rollout;
mod rollout_strategy;
mod scenarios;
mod ui_state;
