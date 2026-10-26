//! hawier-ui as a library: lets integration tests boot the console router
//! in-process next to an in-process hawier-server.

pub mod config;
pub mod error;
pub mod hawkbit;
pub mod help;
#[cfg(test)]
mod help_audit;
pub mod html;
pub mod logs;
pub mod resources;
pub mod routes;
pub mod searches;
pub mod session;
pub mod state;
pub mod ui_api;
pub mod view;
pub mod views;
