//! The content-addressed layout, shared by the host and every storage
//! extension that keeps a tree.
//!
//! `{tenant}/{sha1[-4..-2]}/{sha1[-2..]}/{sha1}`, tenant upper-cased —
//! hawkBit's `FileArtifactStorage` layout, so a stock hawkBit can share the
//! same directory during the dev stage.

use crate::error::{AppError, AppResult};

/// A sha1 that will be used as a name must be hex and long enough to fan
/// out, or it is a path traversal waiting to happen.
pub fn validate(sha1: &str) -> AppResult<()> {
    if sha1.len() < 4 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(format!("invalid sha1 '{sha1}'")));
    }
    Ok(())
}

/// Relative key for an artifact, with `/` separators.
pub fn key(tenant: &str, sha1: &str) -> AppResult<String> {
    validate(sha1)?;
    let folder1 = &sha1[sha1.len() - 4..sha1.len() - 2];
    let folder2 = &sha1[sha1.len() - 2..];
    Ok(format!(
        "{}/{folder1}/{folder2}/{sha1}",
        tenant.to_uppercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_fans_out_on_the_last_four_hex_digits() {
        assert_eq!(
            key("default", "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed").unwrap(),
            "DEFAULT/46/ed/2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
        );
    }

    #[test]
    fn a_traversing_sha1_is_refused() {
        assert!(key("DEFAULT", "../../etc/passwd").is_err());
        assert!(validate("zz").is_err());
    }
}
