//! hawier storage extension: an S3-compatible object store.
//!
//! Answers "where is artifact X" with a presigned URL and nothing else. The
//! credentials live here, the signing happens here, and hawier never sees
//! either — it takes the URL and either redirects a device to it or performs
//! the transfer itself. No byte of any artifact passes through this module.
//!
//! Two ways to get a link. By default it signs locally (SigV4, no network
//! at all). Where a deployment cannot hand out static credentials — a
//! gateway that mints links, an internal signing service, anything backed by
//! short-lived credentials — set `HAWIER_EXT_S3_SIGNER_URL` and it asks that
//! service instead, using the `http_out` capability. The capability is
//! declared only when the URL is configured, so an extension that signs
//! locally cannot reach the network at all.
//!
//! Configure with `HAWIER_EXT_S3_BUCKET`, `HAWIER_EXT_S3_ENDPOINT` (scheme
//! and host only), `HAWIER_EXT_S3_ACCESS_KEY`, `HAWIER_EXT_S3_SECRET_KEY`,
//! and optionally `HAWIER_EXT_S3_REGION`, `HAWIER_EXT_S3_PATH_STYLE`,
//! `HAWIER_EXT_S3_PRESIGN_TTL_SECS`, `HAWIER_EXT_S3_SIGNER_URL`.

mod sigv4;

use hawier_api::ext::{
    points, Capability, ExecutionSite, ExtManifest, ExtRequest, ExtResponse, HttpOutcome,
    HttpRequest, StorageLocate, StorageLocation, StorageOp,
};
use sigv4::Credentials;

struct Config {
    bucket: String,
    endpoint: String,
    credentials: Credentials,
    path_style: bool,
    ttl_secs: u32,
}

fn env(key: &str) -> Result<String, String> {
    std::env::var(key).map_err(|_| format!("{key} is required"))
}

fn config() -> Result<Config, String> {
    Ok(Config {
        bucket: env("HAWIER_EXT_S3_BUCKET")?,
        endpoint: env("HAWIER_EXT_S3_ENDPOINT")?,
        credentials: Credentials {
            access_key: env("HAWIER_EXT_S3_ACCESS_KEY")?,
            secret_key: env("HAWIER_EXT_S3_SECRET_KEY")?,
            region: std::env::var("HAWIER_EXT_S3_REGION")
                .unwrap_or_else(|_| "us-east-1".to_string()),
        },
        path_style: std::env::var("HAWIER_EXT_S3_PATH_STYLE")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(true),
        ttl_secs: std::env::var("HAWIER_EXT_S3_PRESIGN_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900),
    })
}

fn manifest() -> ExtManifest {
    // Asked for only when an external signer is configured: an extension
    // that signs locally has no business reaching the network, and the host
    // enforces exactly what is declared here.
    let capabilities = match signer_host() {
        Some(host) => vec![Capability::HttpOut { hosts: vec![host] }],
        None => Vec::new(),
    };
    ExtManifest {
        name: "hawier-ext-s3".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        points: vec![points::STORAGE_LOCATE.to_string()],
        sites: vec![ExecutionSite::Ddi, ExecutionSite::Control],
        capabilities,
        deadline_ms: 2_000,
    }
}

/// Host of the configured signing service, if there is one.
fn signer_host() -> Option<String> {
    let url = std::env::var("HAWIER_EXT_S3_SIGNER_URL").ok()?;
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    Some(rest.split('/').next()?.to_string())
}

/// Ask the signing service for a link.
///
/// The extension describes the request; the host makes it, and would refuse
/// outright if this reached past the host named in the manifest.
fn signed_by_service(
    endpoint: &str,
    bucket: &str,
    key: &str,
    method: &str,
    ttl: u32,
    tenant: &str,
) -> Result<String, String> {
    let body = serde_json::json!({
        "bucket": bucket,
        "key": key,
        "method": method,
        "expiresIn": ttl,
        "tenant": tenant,
    });
    let outcome = hawier_api::ext::http(&HttpRequest {
        url: endpoint.to_string(),
        method: "POST".to_string(),
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: Some(body.to_string()),
    });
    let response = match outcome {
        HttpOutcome::Ok(response) => response,
        HttpOutcome::Refused { reason } => {
            return Err(format!("the host refused the signing call: {reason}"))
        }
        HttpOutcome::Failed { reason } => return Err(format!("signing service: {reason}")),
    };
    if !(200..300).contains(&response.status) {
        return Err(format!("signing service answered {}", response.status));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| format!("signing service: {e}"))?;
    parsed
        .get("url")
        .and_then(|url| url.as_str())
        .map(str::to_string)
        .ok_or_else(|| "signing service returned no url".to_string())
}

fn call(request: ExtRequest) -> ExtResponse {
    if request.point != points::STORAGE_LOCATE {
        return ExtResponse::Err {
            message: format!("hawier-ext-s3 does not implement {}", request.point),
        };
    }
    let locate = match serde_json::from_value::<StorageLocate>(request.payload) {
        Ok(locate) => locate,
        Err(error) => {
            return ExtResponse::Err {
                message: format!("unusable storage.locate payload: {error}"),
            }
        }
    };
    match config().and_then(|config| locate_artifact(&config, &locate)) {
        Ok(location) => match serde_json::to_value(location) {
            Ok(payload) => ExtResponse::Ok { payload },
            Err(error) => ExtResponse::Err {
                message: format!("cannot encode a location: {error}"),
            },
        },
        Err(message) => ExtResponse::Err { message },
    }
}

/// Same tree as the filesystem extension, so a bucket and a directory hold
/// the same layout and content addressing means one thing everywhere.
fn key_for(tenant: &str, sha1: &str) -> Result<String, String> {
    if sha1.len() < 4 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid sha1 '{sha1}'"));
    }
    Ok(format!(
        "{}/{}/{}/{sha1}",
        tenant.to_uppercase(),
        &sha1[sha1.len() - 4..sha1.len() - 2],
        &sha1[sha1.len() - 2..]
    ))
}

fn locate_artifact(config: &Config, locate: &StorageLocate) -> Result<StorageLocation, String> {
    let key = key_for(&locate.tenant, &locate.sha1)?;
    let method = match locate.op {
        StorageOp::Get => "GET",
        StorageOp::Put => "PUT",
        StorageOp::Head => "HEAD",
        StorageOp::Delete => "DELETE",
    };
    let now = chrono::Utc::now();
    let url = match std::env::var("HAWIER_EXT_S3_SIGNER_URL") {
        Ok(service) if !service.trim().is_empty() => signed_by_service(
            service.trim(),
            &config.bucket,
            &key,
            method,
            config.ttl_secs,
            &locate.tenant,
        )?,
        _ => sigv4::presign(
            &config.credentials,
            method,
            &config.endpoint,
            &config.bucket,
            &key,
            config.ttl_secs,
            now,
            config.path_style,
        )?,
    };
    Ok(StorageLocation::Url {
        url,
        method: method.to_string(),
        headers: Vec::new(),
        expires_at: Some(now.timestamp_millis() + i64::from(config.ttl_secs) * 1000),
        // The point of an object store: hand the device the URL and stay out
        // of the byte path. Only a GET is safe to hand out this way.
        redirect_ok: matches!(locate.op, StorageOp::Get),
    })
}

hawier_api::wasm_extension!(manifest, call);

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn key_matches_the_filesystem_layout() {
        assert_eq!(
            key_for("default", "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed").unwrap(),
            "DEFAULT/46/ed/2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
        );
    }

    #[test]
    fn rejects_a_non_hex_sha1() {
        assert!(key_for("DEFAULT", "../../etc/passwd").is_err());
    }
}
