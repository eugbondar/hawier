//! hawier-client: DDI device simulator (rewrite of hawkbit-sdk-device).
//!
//! Library used by the `hawier-device` binary and by hawier-server's
//! integration tests to drive full client↔DDI cycles.

pub mod client;
pub mod device;

pub use client::{DdiAuth, DdiClient};
pub use device::{CycleOutcome, Device, DeviceOptions};
