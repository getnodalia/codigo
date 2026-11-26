// SPDX-License-Identifier: MIT OR Apache-2.0
//! Which provider, which model and where it lives.

use crate::{Error, Result};

/// Who answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Provider {
    #[default]
    Anthropic,
    OpenAi,
    /// A model running on this computer through Ollama.
    Ollama,
    /// Any service that speaks the OpenAI chat protocol, at an address the person gives.
    Custom,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Anthropic,
        Provider::OpenAi,
        Provider::Ollama,
        Provider::Custom,
    ];

    /// The word stored in settings.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::Ollama => "ollama",
            Provider::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> Provider {
        Provider::ALL
            .into_iter()
            .find(|p| p.id() == id)
            .unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "OpenAI",
            Provider::Ollama => "Ollama (this computer)",
            Provider::Custom => "OpenAI-compatible",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => "claude-sonnet-5-5",
            Provider::OpenAi => "gpt-5",
            Provider::Ollama => "llama3.2",
            Provider::Custom => "",
        }
    }

    pub fn default_base(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Ollama => "http://localhost:11434/v1",
            Provider::Custom => "",
        }
    }

    /// Whether it needs a key (a model on this computer does not).
    pub fn needs_key(self) -> bool {
        matches!(self, Provider::Anthropic | Provider::OpenAi)
    }
}

/// The person's choice, without the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub provider: Provider,
    pub model: String,
    /// Where the provider is; empty for the provider's usual address.
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Config {
        Config::new(Provider::default())
    }
}

impl Config {
    pub fn new(provider: Provider) -> Config {
        Config {
            provider,
            model: provider.default_model().to_owned(),
            base_url: String::new(),
        }
    }

    /// The address requests go to, checked: a key is only ever sent over https, or to this computer.
    pub fn base(&self) -> Result<String> {
        let typed = self.base_url.trim().trim_end_matches('/');
        let base = if typed.is_empty() {
            self.provider.default_base()
        } else {
            typed
        };
        if base.is_empty() {
            return Err(Error::Setup(
                "enter the address of the service in the AI settings".into(),
            ));
        }
        if base.contains(char::is_whitespace) || base.contains('@') {
            return Err(Error::Setup("that address is not valid".into()));
        }
        let loopback = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
            .iter()
            .any(|p| {
                base.strip_prefix(p)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
            });
        if base.starts_with("https://") || loopback {
            Ok(base.to_owned())
        } else {
            Err(Error::Setup(
                "the service must use https:// (http:// only works for this computer)".into(),
            ))
        }
    }

    pub fn model(&self) -> Result<&str> {
        let m = self.model.trim();
        if m.is_empty() {
            Err(Error::Setup("choose a model in the AI settings".into()))
        } else {
            Ok(m)
        }
    }
}
