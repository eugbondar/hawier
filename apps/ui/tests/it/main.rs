//! The console's integration suite: one binary, one module per area.
//!
//! Cargo mints an executable per file directly under `tests/`, and these link
//! hawier-server through the dev-dependency — the whole wasmtime stack — so
//! each file was its own ~470 MB copy of the same closure.
//!
//! Everything here is `#[ignore]`d and wants a real Postgres: `make it`.
//! The audit alone, which is the loop for anything touching `apps/ui` or
//! `static/assets`, is `make ui-audit`.

mod common;

mod ui;
mod ui_audit;
