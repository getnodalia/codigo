// SPDX-License-Identifier: MIT OR Apache-2.0
//! Signing in with GitHub's device flow: the person is shown a short code, enters it on
//! github.com in their browser, and Código polls until GitHub hands over a token. It needs no
//! secret of ours and no server to receive a redirect. It does need an OAuth app's client id,
//! which is not a secret; Código never ships someone else's, so the id comes from the build
//! (`CODIGO_GITHUB_CLIENT_ID`) or the person's own settings.

use serde_json::{json, Value};

use crate::http::{request, Method};
use crate::{Error, Result};

/// What to show the person while they authorise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds to wait between polls.
    pub interval: u64,
    /// Seconds until the code stops working.
    pub expires_in: u64,
}

/// What a poll found.
#[derive(Clone, PartialEq, Eq)]
pub enum Poll {
    Pending,
    /// Ask less often.
    SlowDown,
    Token(String),
    Denied,
    Expired,
}

impl std::fmt::Debug for Poll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Poll::Token(_) => f.write_str("Poll::Token(…)"),
            Poll::Pending => f.write_str("Poll::Pending"),
            Poll::SlowDown => f.write_str("Poll::SlowDown"),
            Poll::Denied => f.write_str("Poll::Denied"),
            Poll::Expired => f.write_str("Poll::Expired"),
        }
    }
}

/// The scopes Código asks for: repositories (clone, pull requests, reviews) and the profile.
pub const SCOPES: &str = "repo read:user";

fn check_client_id(id: &str) -> Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
        return Err(Error::Unexpected("the OAuth client id is not valid".into()));
    }
    Ok(())
}

/// Start signing in.
pub fn start(client_id: &str) -> Result<DeviceCode> {
    check_client_id(client_id)?;
    let v = request(
        Method::Post,
        "https://github.com/login/device/code",
        None,
        Some(&json!({ "client_id": client_id, "scope": SCOPES })),
    )?;
    parse_code(&v)
}

pub fn parse_code(v: &Value) -> Result<DeviceCode> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let num = |k: &str, d: u64| v.get(k).and_then(Value::as_u64).unwrap_or(d);
    if let Some(e) = text("error") {
        return Err(Error::Api(text("error_description").unwrap_or(e)));
    }
    match (
        text("device_code"),
        text("user_code"),
        text("verification_uri"),
    ) {
        (Some(device_code), Some(user_code), Some(verification_uri)) => Ok(DeviceCode {
            device_code,
            user_code,
            verification_uri,
            interval: num("interval", 5).clamp(1, 60),
            expires_in: num("expires_in", 900).clamp(1, 3600),
        }),
        _ => Err(Error::Unexpected(
            "the sign-in answer is missing its code".into(),
        )),
    }
}

/// Ask whether the person has authorised yet.
pub fn poll(client_id: &str, device_code: &str) -> Result<Poll> {
    check_client_id(client_id)?;
    let v = request(
        Method::Post,
        "https://github.com/login/oauth/access_token",
        None,
        Some(&json!({
            "client_id": client_id,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        })),
    )?;
    Ok(parse_poll(&v))
}

pub fn parse_poll(v: &Value) -> Poll {
    if let Some(t) = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        return Poll::Token(t.to_owned());
    }
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") | None => Poll::Pending,
        Some("slow_down") => Poll::SlowDown,
        Some("access_denied") => Poll::Denied,
        _ => Poll::Expired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_answer_is_read_and_bounded() {
        let v = json!({"device_code": "dc", "user_code": "ABCD-1234", "verification_uri": "https://github.com/login/device", "interval": 0, "expires_in": 999999});
        let c = parse_code(&v).expect("code");
        assert_eq!(
            (c.user_code.as_str(), c.interval, c.expires_in),
            ("ABCD-1234", 1, 3600)
        );
        assert!(parse_code(&json!({})).is_err());
        assert_eq!(
            parse_code(
                &json!({"error": "device_flow_disabled", "error_description": "Device flow must be enabled"})
            ),
            Err(Error::Api("Device flow must be enabled".into()))
        );
    }

    #[test]
    fn every_poll_answer_has_a_meaning_and_the_token_stays_hidden() {
        assert_eq!(
            parse_poll(&json!({"error": "authorization_pending"})),
            Poll::Pending
        );
        assert_eq!(parse_poll(&json!({"error": "slow_down"})), Poll::SlowDown);
        assert_eq!(parse_poll(&json!({"error": "access_denied"})), Poll::Denied);
        assert_eq!(
            parse_poll(&json!({"error": "expired_token"})),
            Poll::Expired
        );
        let t = parse_poll(&json!({"access_token": "gho_secret", "token_type": "bearer"}));
        assert_eq!(t, Poll::Token("gho_secret".into()));
        assert!(!format!("{t:?}").contains("gho_"));
    }

    #[test]
    fn a_client_id_cannot_inject_anything() {
        assert!(start("").is_err());
        assert!(start("a b").is_err());
        assert!(poll("x\"y", "d").is_err());
    }
}
