//! Build a provider client from its configuration.

use crate::base::{KeyLookup, Provider};
use crate::google::GoogleProvider;
use crate::openai_compat::OpenAiCompatProvider;
use crate::types::{ProviderConfig, ProviderKind};

/// Create the client for `config`. Keys are looked up per request, never stored.
///
/// The match is exhaustive over [`ProviderKind`], so a new kind cannot be configured without a
/// client for it.
#[must_use]
pub fn build_provider(config: &ProviderConfig, key_lookup: &KeyLookup) -> Box<dyn Provider> {
    let lookup = KeyLookup::clone(key_lookup);
    match config.kind {
        ProviderKind::OpenAiCompat => Box::new(OpenAiCompatProvider::new(config.clone(), lookup)),
        ProviderKind::Google => Box::new(GoogleProvider::new(config.clone(), lookup)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn every_kind_builds_a_client_with_the_right_capabilities() {
        let lookup: KeyLookup = Arc::new(|_| None);
        for kind in ProviderKind::ALL {
            let config = ProviderConfig {
                name: format!("p_{}", kind.as_str()),
                kind,
                base_url: "https://example.com/v1".to_owned(),
                model: "m".to_owned(),
                timeout_s: 5.0,
                keyring_name: None,
            };
            let provider = build_provider(&config, &lookup);
            assert_eq!(provider.name(), config.name);
            assert_eq!(
                provider.supports_tools(),
                kind == ProviderKind::OpenAiCompat
            );
        }
    }
}
