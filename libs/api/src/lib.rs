//! hawier-api: shared wire types for hawier.
//!
//! Pure serde DTOs and hawkBit-compatible enum integer codes. No IO here.
//! This is deliberately the ONLY shared library crate in the workspace —
//! do not split further (project constraint: no micro-libraries).

pub mod codes;
pub mod ddi;
pub mod ext;
pub mod ui_state;
