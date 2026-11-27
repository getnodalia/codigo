// SPDX-License-Identifier: MIT OR Apache-2.0
//! One HTTPS request to a GitLab server, and what its status means.

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
        // A redirect would carry the token to wherever it points: none is followed.
        .max_redirects(0)
        // Statuses are read here: a refusal explains itself in its body.
        .http_status_as_error(false)
        .build()
        .into()
}

pub enum Method {
    Get,
    Post,
}

/// The URL of an API path on `host`. Only `https://{host}/api/v4/…` is ever built, so the token
/// goes nowhere else.
pub fn url(host: &str, path: &str) -> String {
    format!("https://{host}/api/v4/{}", path.trim_start_matches('/'))
}

/// The answer of a request: its JSON (`Null` when empty) and the page after this one, if any.
pub struct Answer {
    pub value: Value,
    pub next_page: Option<u32>,
}

/// Send a request and read what it answers.
pub fn request(
    method: Method,
    host: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Result<Answer> {
    let url = url(host, path);
    let agent = agent();
    let mut response = match method {
        Method::Get => {
            let mut r = agent.get(&url).header("User-Agent", USER_AGENT);
            if let Some(t) = token {
                r = r.header("PRIVATE-TOKEN", t);
            }
            r.call()
        }
        Method::Post => {
            let mut r = agent.post(&url).header("User-Agent", USER_AGENT);
            if let Some(t) = token {
                r = r.header("PRIVATE-TOKEN", t);
            }
            match body {
                Some(b) => r.send_json(b),
                None => r.send_empty(),
            }
        }
    }
    .map_err(|e| Error::Network(scrub(&e.to_string(), token)))?;
    let status = response.status().as_u16();
    let next_page = response
        .headers()
        .get("x-next-page")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u32>().ok());
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
    map_status(status, value).map(|value| Answer { value, next_page })
}

/// What a status code and body come to.
pub fn map_status(status: u16, value: Value) -> Result<Value> {
    match status {
        200..=299 => Ok(value),
        301..=308 => Err(Error::Unexpected(
            "the server redirected the request; check the address".into(),
        )),
        401 => Err(Error::Unauthorized),
        429 => Err(Error::RateLimited),
        404 => Err(Error::NotFound),
        _ => {
            let message = ["message", "error"]
                .iter()
                .find_map(|k| value.get(*k))
                .map(|m| match m {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "the request was refused".to_owned());
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
        assert!(map_status(200, json!({})).is_ok());
        assert!(map_status(201, json!(null)).is_ok());
        assert_eq!(map_status(401, json!({})), Err(Error::Unauthorized));
        assert_eq!(map_status(404, json!({})), Err(Error::NotFound));
        assert_eq!(map_status(429, json!({})), Err(Error::RateLimited));
        assert!(matches!(
            map_status(302, json!(null)),
            Err(Error::Unexpected(_))
        ));
        assert_eq!(
            map_status(403, json!({"message": "403 Forbidden"})),
            Err(Error::Api("403 Forbidden".into()))
        );
        assert_eq!(
            map_status(400, json!({"error": "title is missing"})),
            Err(Error::Api("title is missing".into()))
        );
        assert_eq!(
            map_status(422, json!(null)),
            Err(Error::Api("the request was refused".into()))
        );
    }

    #[test]
    fn urls_are_only_ever_built_on_the_given_host() {
        assert_eq!(url("gitlab.com", "/user"), "https://gitlab.com/api/v4/user");
        assert_eq!(
            url("git.example.org:8443", "projects?membership=true"),
            "https://git.example.org:8443/api/v4/projects?membership=true"
        );
    }

    #[test]
    fn the_token_is_scrubbed_from_error_text() {
        assert_eq!(
            scrub("bad glpat-abcdefgh here", Some("glpat-abcdefgh")),
            "bad *** here"
        );
        assert_eq!(scrub("short", Some("abc")), "short");
    }
}
