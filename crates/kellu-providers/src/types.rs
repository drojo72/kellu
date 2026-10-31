//! Provider-neutral data types. No dependencies.

/// The wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// OpenAI-compatible chat completions (OpenRouter, DeepSeek, xAI, local runtimes).
    OpenAiCompat,
    /// Google's Gemini API (also serves Gemma).
    Google,
}

impl ProviderKind {
    /// Every kind, sorted by configuration name.
    pub const ALL: [Self; 2] = [Self::Google, Self::OpenAiCompat];

    /// The name used in `config.toml`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompat => "openai_compat",
            Self::Google => "google",
        }
    }

    /// Parse a configuration name (exact match).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// One model provider.
///
/// `keyring_name` names a keyring entry, never the secret itself. `None` means the endpoint needs
/// no authentication (a local runtime).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderConfig {
    /// Name from the configuration (`[providers.<name>]`).
    pub name: String,
    /// Wire protocol.
    pub kind: ProviderKind,
    /// Endpoint base URL without a trailing slash.
    pub base_url: String,
    /// Model identifier sent to the provider.
    pub model: String,
    /// Per-request timeout in seconds.
    pub timeout_s: f64,
    /// Keyring name (`openrouter` resolves to the entry `ai_openrouter_api_key`).
    pub keyring_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_name() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn all_is_sorted_by_name() {
        let names: Vec<_> = ProviderKind::ALL.iter().map(|k| k.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn unknown_or_differently_cased_names_are_rejected() {
        for name in ["", "openai", "Google", "OPENAI_COMPAT", "google "] {
            assert_eq!(ProviderKind::parse(name), None, "{name:?}");
        }
    }
}
