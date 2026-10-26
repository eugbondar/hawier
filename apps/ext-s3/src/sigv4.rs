//! AWS Signature Version 4 — presigned URLs.
//!
//! Lives in the extension, not in hawier: the credentials are the
//! extension's, the signing is the extension's, and the node never needs
//! either. All hawier gets back is a URL.
//!
//! Written out rather than pulled in: presigning is a canonical request plus
//! four nested HMACs, and the HMAC is `hawier_api::ext::hmac_sha256`. The AWS
//! SDK and `object_store` both bring a large tree for this one function.

use chrono::{DateTime, Utc};
use hawier_api::ext::hmac_sha256;
use sha2::{Digest, Sha256};

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "s3";
/// Presigned URLs carry no body hash: the signature covers the URL only.
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
}

/// Percent-encoding per AWS: unreserved characters pass, everything else is
/// `%XX` uppercase. Path segments keep `/`, query components do not.
fn uri_encode(value: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn split_endpoint(endpoint: &str) -> Result<(&str, &str), String> {
    let trimmed = endpoint.trim_end_matches('/');
    let (scheme, authority) = trimmed
        .split_once("://")
        .ok_or_else(|| format!("S3 endpoint '{endpoint}' must include a scheme"))?;
    if authority.is_empty() {
        return Err(format!("S3 endpoint '{endpoint}' has no host"));
    }
    Ok((scheme, authority))
}

/// Build a presigned URL valid for `expires_in` seconds.
///
/// `now` is injected rather than read from the clock so the signature is a
/// pure function of its inputs and can be checked against AWS's published
/// example.
#[allow(clippy::too_many_arguments)]
pub fn presign(
    credentials: &Credentials,
    method: &str,
    endpoint: &str,
    bucket: &str,
    key: &str,
    expires_in: u32,
    now: DateTime<Utc>,
    path_style: bool,
) -> Result<String, String> {
    let (scheme, authority) = split_endpoint(endpoint)?;
    let (host, path) = if path_style {
        (authority.to_string(), format!("/{bucket}/{key}"))
    } else {
        (format!("{bucket}.{authority}"), format!("/{key}"))
    };
    let canonical_uri = uri_encode(&path, false);

    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let scope = format!("{date}/{}/{SERVICE}/aws4_request", credentials.region);

    // Canonical query: sorted by key, every component percent-encoded.
    let mut params = [
        ("X-Amz-Algorithm".to_string(), ALGORITHM.to_string()),
        (
            "X-Amz-Credential".to_string(),
            format!("{}/{scope}", credentials.access_key),
        ),
        ("X-Amz-Date".to_string(), amz_date.clone()),
        ("X-Amz-Expires".to_string(), expires_in.to_string()),
        ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
    ];
    params.sort();
    let canonical_query = params
        .iter()
        .map(|(k, v)| format!("{}={}", uri_encode(k, true), uri_encode(v, true)))
        .collect::<Vec<_>>()
        .join("&");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\nhost:{host}\n\nhost\n{UNSIGNED_PAYLOAD}"
    );
    let string_to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let k_date = hmac_sha256(
        format!("AWS4{}", credentials.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, credentials.region.as_bytes());
    let k_service = hmac_sha256(&k_region, SERVICE.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    Ok(format!(
        "{scheme}://{host}{canonical_uri}?{canonical_query}&X-Amz-Signature={signature}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// AWS's own worked example for a presigned GET (query-parameter form).
    /// A known answer, not a self-consistency check.
    #[test]
    fn matches_the_aws_documented_example() {
        let credentials = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            region: "us-east-1".to_string(),
        };
        let url = presign(
            &credentials,
            "GET",
            "https://s3.amazonaws.com",
            "examplebucket",
            "test.txt",
            86400,
            Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap(),
            false,
        )
        .unwrap();
        assert!(url.starts_with("https://examplebucket.s3.amazonaws.com/test.txt?"));
        assert!(url.ends_with(
            "&X-Amz-Signature=\
             aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        ));
    }

    #[test]
    fn path_style_puts_the_bucket_in_the_path() {
        let credentials = Credentials {
            access_key: "minio".to_string(),
            secret_key: "minio123".to_string(),
            region: "us-east-1".to_string(),
        };
        let url = presign(
            &credentials,
            "PUT",
            "http://minio:9000",
            "artifacts",
            "DEFAULT/ab/cd/abcd",
            900,
            Utc.with_ymd_and_hms(2026, 9, 5, 12, 0, 0).unwrap(),
            true,
        )
        .unwrap();
        assert!(url.starts_with("http://minio:9000/artifacts/DEFAULT/ab/cd/abcd?"));
        let signature = url.rsplit("X-Amz-Signature=").next().unwrap();
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn encodes_reserved_characters_but_keeps_path_separators() {
        assert_eq!(uri_encode("a b/c", false), "a%20b/c");
        assert_eq!(uri_encode("a b/c", true), "a%20b%2Fc");
        assert_eq!(uri_encode("~-_.", true), "~-_.");
    }

    #[test]
    fn rejects_an_endpoint_without_a_scheme() {
        assert!(split_endpoint("minio:9000").is_err());
    }
}
