// SPDX-License-Identifier: MIT OR Apache-2.0
//! One question to a provider and the answer.

use std::time::Duration;

use serde_json::{json, Value};

use crate::{Config, Error, Provider, Result};

const USER_AGENT: &str = concat!("Codigo/", env!("CARGO_PKG_VERSION"));
const MAX_BODY: u64 = 8 * 1024 * 1024;
const MAX_TOKENS: u32 = 4096;
/// The most text of the conversation sent in one request.
const MAX_INPUT: usize = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn wire(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            text: text.into(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            text: text.into(),
        }
    }
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(180)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// The newest messages that fit, oldest dropped first; the last message is always kept.
fn fit(messages: &[Message]) -> Vec<&Message> {
    let mut kept: Vec<&Message> = Vec::new();
    let mut size = 0usize;
    for m in messages.iter().rev() {
        size += m.text.len();
        if size > MAX_INPUT && !kept.is_empty() {
            break;
        }
        kept.push(m);
    }
    kept.reverse();
    // A conversation has to open with the person.
    while kept.first().is_some_and(|m| m.role == Role::Assistant) {
        kept.remove(0);
    }
    kept
}

/// What a request to this provider looks like: where, which headers, which body.
pub struct Request {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

pub fn build(
    config: &Config,
    key: Option<&str>,
    system: &str,
    messages: &[Message],
) -> Result<Request> {
    let base = config.base()?;
    let model = config.model()?.to_owned();
    let kept = fit(messages);
    if kept.is_empty() {
        return Err(Error::Setup("write something to ask first".into()));
    }
    if config.provider.needs_key() && key.is_none() {
        return Err(Error::Setup(format!(
            "add your {} key in the AI settings first",
            config.provider.name()
        )));
    }
    let list: Vec<Value> = kept
        .iter()
        .map(|m| json!({ "role": m.role.wire(), "content": m.text }))
        .collect();
    Ok(match config.provider {
        Provider::Anthropic => {
            let mut headers = vec![("anthropic-version", "2023-06-01".to_owned())];
            if let Some(k) = key {
                headers.push(("x-api-key", k.to_owned()));
            }
            Request {
                url: format!("{base}/v1/messages"),
                headers,
                body: json!({
                    "model": model,
                    "max_tokens": MAX_TOKENS,
                    "system": system,
                    "messages": list,
                }),
            }
        }
        Provider::OpenAi | Provider::Ollama | Provider::Custom => {
            let mut all = vec![json!({ "role": "system", "content": system })];
            all.extend(list);
            let headers = key
                .map(|k| vec![("Authorization", format!("Bearer {k}"))])
                .unwrap_or_default();
            Request {
                url: format!("{base}/chat/completions"),
                headers,
                body: json!({ "model": model, "messages": all }),
            }
        }
    })
}

/// The answer's text, from either provider's reply.
pub fn read_reply(provider: Provider, value: &Value) -> Result<String> {
    let text = match provider {
        Provider::Anthropic => value.get("content").and_then(Value::as_array).map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        }),
        _ => value
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t),
        _ => Err(Error::Unexpected("the answer was empty".into())),
    }
}

/// Ask the provider and wait for the whole answer.
pub fn complete(
    config: &Config,
    key: Option<&str>,
    system: &str,
    messages: &[Message],
) -> Result<String> {
    let request = build(config, key, system, messages)?;
    let scrub = |t: &str| match key {
        Some(k) if k.len() >= 6 => t.replace(k, "***"),
        _ => t.to_owned(),
    };
    let mut call = agent()
        .post(&request.url)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/json");
    for (name, value) in &request.headers {
        call = call.header(*name, value.as_str());
    }
    let mut response = call
        .send_json(&request.body)
        .map_err(|e| Error::Network(scrub(&e.to_string())))?;
    let status = response.status().as_u16();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|e| Error::Network(scrub(&e.to_string())))?;
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    match status {
        200..=299 => read_reply(config.provider, &value),
        401 | 403 => Err(Error::Unauthorized),
        429 => Err(Error::RateLimited),
        _ => {
            let message = value
                .get("error")
                .and_then(|e| e.get("message").and_then(Value::as_str).or(e.as_str()))
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or("the request was refused");
            Err(Error::Api(scrub(message)))
        }
    }
}
