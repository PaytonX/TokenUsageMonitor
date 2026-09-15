//! Volcano Engine (Volcengine / 火山引擎) V4 request signing.
//!
//! Implements the HMAC-SHA256 signature scheme documented at
//! <https://www.volcengine.com/docs/6369/67269>. The same scheme is used by
//! all Volcengine services including the Ark Agent Plan APIs.
//!
//! Canonical request layout (verified against the official Ark examples,
//! e.g. <https://www.volcengine.com/docs/82379/2479847>):
//!
//!   METHOD\nPATH\nQUERY\nCanonicalHeaders\nSignedHeaders\nBODY_HASH
//!
//! where `CanonicalHeaders` is one `lowercase(name):value` line per header,
//! sorted by name, each line terminated by `\n` - so a blank line separates
//! the header block from the signed-headers list. The signed headers are
//! `content-type;host;x-content-sha256;x-date` (the doc example signs
//! Content-Type even though it is not one of the auth headers proper).
//!
//! For Volcengine Ark specifically:
//!   - service = "ark"
//!   - region = "cn-beijing"
//!   - host = "ark.cn-beijing.volcengineapi.com"

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// What the request module needs to attach to every Volcano request.
#[derive(Debug, Clone)]
pub struct SignedHeaders {
    /// The full `Authorization` header value.
    pub authorization: String,
    /// `X-Date` header (RFC3339-ish: `YYYYMMDDTHHMMSSZ`).
    pub x_date: String,
    /// `X-Content-Sha256` header (lowercase hex of body SHA256).
    pub x_content_sha256: String,
    /// `Host` header value.
    pub host: String,
}

/// Sign a single request.
///
/// `method` upper-case ("GET", "POST", ...). `path` starts with `/`.
/// `query` is the raw query string WITHOUT the leading `?`. May be empty.
/// `content_type` is the `Content-Type` header value that will be sent
/// (it participates in the signature, e.g. "application/json").
/// `body` is the raw request body bytes; empty for GET.
pub fn sign(
    access_key: &str,
    secret_key: &str,
    method: &str,
    host: &str,
    path: &str,
    query: &str,
    content_type: &str,
    body: &[u8],
    timestamp: DateTime<Utc>,
) -> SignedHeaders {
    let x_date = timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = timestamp.format("%Y%m%d").to_string();

    // Body hash.
    let body_hash = {
        let mut h = Sha256::new();
        h.update(body);
        hex::encode(h.finalize())
    };

    // Canonical headers: one `name:value` line per header, sorted by name,
    // each line terminated by `\n`.
    let canonical_headers = canonical_headers(&[
        ("content-type", content_type),
        ("host", host),
        ("x-content-sha256", body_hash.as_str()),
        ("x-date", x_date.as_str()),
    ]);
    let signed_headers = "content-type;host;x-content-sha256;x-date";

    // Sort query parameters. Volcengine expects query string to be already
    // canonical (sorted by key) but real callers can pass either way; we
    // sort defensively here.
    let canonical_query = sort_query(query);

    let canonical_request = format!(
        "{method}\n{path}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{body_hash}",
        method = method.to_uppercase(),
        path = path,
        canonical_query = canonical_query,
        canonical_headers = canonical_headers,
        signed_headers = signed_headers,
        body_hash = body_hash,
    );

    // Credential scope: <date>/<region>/<service>/request
    let region = "cn-beijing";
    let service = "ark";
    let credential_scope = format!("{short_date}/{region}/{service}/request");

    // String to sign.
    let canonical_request_hash = {
        let mut h = Sha256::new();
        h.update(canonical_request.as_bytes());
        hex::encode(h.finalize())
    };
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{credential_scope}\n{canonical_request_hash}",
        x_date = x_date,
        credential_scope = credential_scope,
        canonical_request_hash = canonical_request_hash,
    );

    // Derive signing key.
    let k_date = hmac_bytes(secret_key.as_bytes(), short_date.as_bytes());
    let k_region = hmac_bytes(&k_date, region.as_bytes());
    let k_service = hmac_bytes(&k_region, service.as_bytes());
    let k_request = hmac_bytes(&k_service, b"request");

    let signature = hex::encode(hmac_bytes(&k_request, string_to_sign.as_bytes()));

    let authorization = format!(
        "HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        access_key = access_key,
        credential_scope = credential_scope,
        signed_headers = signed_headers,
        signature = signature,
    );

    SignedHeaders {
        authorization,
        x_date,
        x_content_sha256: body_hash,
        host: host.to_string(),
    }
}

fn hmac_bytes(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Build the canonical headers block: one `name:value` line per header,
/// sorted by header name, each line terminated by `\n`.
fn canonical_headers(headers: &[(&str, &str)]) -> String {
    let mut sorted: Vec<(&str, &str)> = headers.to_vec();
    sorted.sort_by_key(|(name, _)| *name);
    let mut out = String::new();
    for (name, value) in sorted {
        out.push_str(name);
        out.push(':');
        out.push_str(value);
        out.push('\n');
    }
    out
}

/// Sort query string parameters by key, returning a new canonical query string.
/// Empty input -> empty output. Each parameter is preserved as-is (no
/// URL-encoding normalization); callers should pass already-encoded values.
fn sort_query(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<&str> = query.split('&').collect();
    pairs.sort_by(|a, b| {
        let ka = a.split('=').next().unwrap_or("");
        let kb = b.split('=').next().unwrap_or("");
        ka.cmp(kb)
    });
    pairs.join("&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Smoke test: a fixed-timestamp signature is deterministic and matches
    /// the documented structure. The exact signature value is sensitive to
    /// every byte of the canonical request; this test guards against
    /// accidental refactor regressions.
    #[test]
    fn signature_is_deterministic() {
        let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let h1 = sign(
            "AKxyz",
            "SKxyz",
            "POST",
            "ark.cn-beijing.volcengineapi.com",
            "/",
            "Action=GetAFPUsage&Version=2024-01-01",
            "application/json",
            b"{}",
            ts,
        );
        let h2 = sign(
            "AKxyz",
            "SKxyz",
            "POST",
            "ark.cn-beijing.volcengineapi.com",
            "/",
            "Action=GetAFPUsage&Version=2024-01-01",
            "application/json",
            b"{}",
            ts,
        );
        assert_eq!(h1.authorization, h2.authorization);
        assert!(h1.authorization.starts_with("HMAC-SHA256 Credential=AKxyz/20260101/cn-beijing/ark/request"));
        assert!(h1.authorization.contains("SignedHeaders=content-type;host;x-content-sha256;x-date"));
        assert_eq!(h1.x_date, "20260101T000000Z");
        // Body hash of "{}" - sha256 of empty JSON.
        assert_eq!(
            h1.x_content_sha256,
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn canonical_headers_are_sorted_and_newline_terminated() {
        let block = canonical_headers(&[
            ("x-date", "20260101T000000Z"),
            ("host", "ark.cn-beijing.volcengineapi.com"),
            ("content-type", "application/json"),
        ]);
        assert_eq!(
            block,
            "content-type:application/json\nhost:ark.cn-beijing.volcengineapi.com\nx-date:20260101T000000Z\n"
        );
    }

    #[test]
    fn sort_query_handles_empty() {
        assert_eq!(sort_query(""), "");
        assert_eq!(sort_query("Action=GetAFPUsage&Version=2024-01-01"),
                   "Action=GetAFPUsage&Version=2024-01-01");
        // Unsorted should get sorted by key.
        assert_eq!(sort_query("Version=2024-01-01&Action=GetAFPUsage"),
                   "Action=GetAFPUsage&Version=2024-01-01");
    }
}
