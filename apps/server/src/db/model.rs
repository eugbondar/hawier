//! Row models for the hawkBit-compatible schema.
//!
//! NARROW: timestamps are BIGINT epoch millis, `optlock_revision` is JPA
//! optimistic-locking baggage (we write 1 on insert and bump on update to
//! stay hawkBit-readable), booleans are nullable in DDL though the code
//! treats them as two-valued. See SCHEMA_NOTES.md.

pub use hawier_api::codes::{
    ActionStatus, ActionType, RolloutGroupStatus, RolloutStatus, TargetUpdateStatus,
};

/// `sp_target` row (the device).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TargetRow {
    pub id: i64,
    pub tenant: String,
    pub controller_id: String,
    pub name: String,
    pub description: Option<String>,
    pub sec_token: String,
    pub address: Option<String>,
    pub last_target_query: Option<i64>,
    pub install_date: Option<i64>,
    pub request_controller_attributes: bool,
    pub update_status: i32,
    pub target_group: Option<String>,
    pub assigned_distribution_set: Option<i64>,
    pub installed_distribution_set: Option<i64>,
    pub target_type: Option<i64>,
    pub created_at: Option<i64>,
    pub created_by: Option<String>,
    pub last_modified_at: Option<i64>,
    pub last_modified_by: Option<String>,
}

/// Principal recorded in `created_by` for device-initiated writes —
/// mirrors hawkBit's plug-and-play controller principal.
pub const CONTROLLER_PRINCIPAL: &str = "CONTROLLER_PLUG_AND_PLAY";

/// Principal for hawier's own background engines.
pub const SYSTEM_PRINCIPAL: &str = "system";

/// Generate a target security token: 16 random bytes, lowercase hex
/// (matches hawkBit's `SecurityTokenGenerator`).
pub fn generate_sec_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn sec_token_is_32_lowercase_hex() {
        let token = super::generate_sec_token();
        assert_eq!(token.len(), 32);
        assert!(token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
