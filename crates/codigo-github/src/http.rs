// SPDX-License-Identifier: MIT OR Apache-2.0
//! One HTTPS request to GitHub, and what its status means.

use std::time::Duration;

use serde_json::Value;

use crate::{Error, Result};

const USER_AGENT: &str = concat!("Codigo/", env!("CARGO_PKG_VERSION"));
/// The largest answer read.
const MAX_BODY: u64 = 16 * 1024 * 1024;

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(30)))
        // Statuses are read here: a refusal explains itself in its body.
        .http_status_as_error(false)
        .build()
        .into()
}

pub enum Method {
    Get,
    Post,
}

/// Only GitHub's own hosts get a token.
fn check_host(url: &str) -> Result<()> {
    let ok = url.starts_with("https://api.github.com/") || url.starts_with("https://github.com/");
    if ok {
        Ok(())
    } else {
        Err(Error::Unexpected(
            "refusing to send a request outside GitHub".into(),
        ))
    }
}

/// Send a request and return the JSON it answers with (`Null` for an empty answer).
pub fn request(
    method: Method,
    url: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Result<Value> {
    check_host(url)?;
    let agent = agent();
    let mut response = match method {
        Method::Get => {
            let mut r = agent
                .get(url)
                .header("User-Agent", USER_AGENT)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(t) = token {
                r = r.header("Authorization", format!("Bearer {t}"));
            }
            r.call()
        }
        Method::Post => {
            let mut r = agent
                .post(url)
                .header("User-Agent", USER_AGENT)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(t) = token {
                r = r.header("Authorization", format!("Bearer {t}"));
            }
            match body {
                Some(b) => r.send_json(b),
                None => r.send_empty(),
            }
        }
    }
    .map_err(|e| Error::Network(scrub(&e.to_string(), token)))?;
    let status = response.status().as_u16();
    let remaining = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|e| Error::Network(scrub(&e.to_string(), token)))?;
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    map_status(status, remaining.as_deref(), value)
}

/// What a status code and body come to.
pub fn map_status(status: u16, rate_remaining: Option<&str>, value: Value) -> Result<Value> {
    match status {
        200..=299 => Ok(value),
        401 => Err(Error::Unauthorized),
        403 | 429 if rate_remaining == Some("0") || status == 429 => Err(Error::RateLimited),
        404 => Err(Error::NotFound),
        _ => {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the request was refused")
                .to_owned();
            Err(Error::Api(message))
        }
    }
}

/// The token is never part of an error text.
fn scrub(text: &str, token: Option<&str>) -> String {
    match token {
        Some(t) if t.len() >= 6 => text.replace(t, "***"),
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn statuses_become_the_right_errors() {
        assert!(map_status(200, None, json!({})).is_ok());
        assert_eq!(map_status(401, None, json!({})), Err(Error::Unauthorized));
        assert_eq!(map_status(404, None, json!({})), Err(Error::NotFound));
        assert_eq!(
            map_status(403, Some("0"), json!({})),
            Err(Error::RateLimited)
        );
        assert_eq!(map_status(429, None, json!({})), Err(Error::RateLimited));
        assert_eq!(
            map_status(
                403,
                Some("12"),
                json!({"message": "Resource not accessible"})
            ),
            Err(Error::Api("Resource not accessible".into()))
        );
        assert_eq!(
            map_status(422, None, json!(null)),
            Err(Error::Api("the request was refused".into()))
        );
    }

    #[test]
    fn a_token_only_goes_to_githubs_own_hosts() {
        assert!(check_host("https://api.github.com/user").is_ok());
        assert!(check_host("https://github.com/login/device/code").is_ok());
        for bad in [
            "https://api.github.com.evil.example/user",
            "http://api.github.com/user",
            "https://example.com/",
            "https://github.com@evil.example/",
        ] {
            assert!(check_host(bad).is_err(), "{bad}");
        }
        assert_eq!(
            scrub("bad ghp_abcdefgh here", Some("ghp_abcdefgh")),
            "bad *** here"
        );
    }
}
