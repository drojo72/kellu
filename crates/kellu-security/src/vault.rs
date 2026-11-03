//! The keyring vault: named secrets with tiers.
//!
//! What the assistant may reach is decided here. The only read the vault offers is
//! [`Vault::provider_key`], and it can only ever ask the store for entries in the `ai_*` space
//! (tier `provider`). There is no call that returns an `infra` or `db` secret: those tiers can be
//! stored, listed and removed, but not read back by the core until a later milestone adds an
//! explicit, confirmed path for them.

use std::sync::Arc;

use kellu_common::{KelluError, Result};
use secrecy::{ExposeSecret, SecretString};

use crate::store::{MAX_SECRET_BYTES, SecretStore};
use crate::tier::{Tier, provider_entry, resolve_entry, tier_of};

/// One entry as shown by `kellu keys list`: never a value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyInfo {
    /// Entry name.
    pub name: String,
    /// Tier deduced from the name.
    pub tier: Tier,
    /// Whether it still carries the earlier prototype's attribute.
    pub legacy: bool,
}

/// What `cleanup-legacy` found or did for one old entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyEntry {
    /// Entry name.
    pub name: String,
    /// Whether a Kellu entry with the same name exists (so the old one is redundant).
    pub migrated: bool,
    /// Whether the old entry was deleted by this run.
    pub removed: bool,
}

/// Named secrets with tiers, on top of a [`SecretStore`].
#[derive(Clone)]
pub struct Vault {
    store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault").finish_non_exhaustive()
    }
}

impl Vault {
    /// A vault over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// The API key of the provider whose `keyring_name` is given.
    ///
    /// The name is mapped to `ai_<keyring_name>_api_key`, so the lookup cannot leave the
    /// `provider` tier whatever the configuration says.
    ///
    /// # Errors
    /// The name is invalid or the store failed. `Ok(None)` means there is no such key.
    pub fn provider_key(&self, keyring_name: &str) -> Result<Option<SecretString>> {
        let entry = provider_entry(keyring_name)?;
        debug_assert_eq!(tier_of(&entry), Tier::Provider);
        self.store.get(&entry)
    }

    /// Store `secret` under `input` (a provider's short name or a full entry name); returns
    /// the entry name used.
    ///
    /// # Errors
    /// The name is invalid, the secret is empty, too long or contains control characters, or
    /// the store refused it.
    pub fn set(&self, input: &str, secret: &SecretString) -> Result<String> {
        let entry = resolve_entry(input)?;
        let value = secret.expose_secret();
        if value.trim().is_empty() {
            return Err(KelluError::config("the secret is empty"));
        }
        if value.len() > MAX_SECRET_BYTES {
            return Err(KelluError::config("the secret is too long"));
        }
        if value.chars().any(char::is_control) {
            return Err(KelluError::config("the secret contains control characters"));
        }
        self.store.set(&entry, secret)?;
        Ok(entry)
    }

    /// Remove the entry `input` (short provider name or full entry name); returns the entry name.
    ///
    /// # Errors
    /// The name is invalid or the store failed.
    pub fn remove(&self, input: &str) -> Result<String> {
        let entry = resolve_entry(input)?;
        self.store.remove(&entry)?;
        Ok(entry)
    }

    /// Every entry Kellu knows about, by tier and name. Values are never loaded.
    ///
    /// # Errors
    /// The store failed.
    pub fn list(&self) -> Result<Vec<KeyInfo>> {
        let mut keys: Vec<KeyInfo> = self
            .store
            .list()?
            .into_iter()
            .map(|entry| KeyInfo {
                tier: tier_of(&entry.name),
                name: entry.name,
                legacy: entry.legacy,
            })
            .collect();
        keys.sort();
        Ok(keys)
    }

    /// Find the entries made by the earlier prototype. With `apply`, delete those that already
    /// exist as Kellu entries; the others are kept (nothing would be left of them).
    ///
    /// # Errors
    /// The store failed.
    pub fn cleanup_legacy(&self, apply: bool) -> Result<Vec<LegacyEntry>> {
        let all = self.list()?;
        let mut report = Vec::new();
        for old in all.iter().filter(|key| key.legacy) {
            let migrated = all.iter().any(|key| !key.legacy && key.name == old.name);
            let removed = apply && migrated && self.store.remove_legacy(&old.name).is_ok();
            report.push(LegacyEntry {
                name: old.name.clone(),
                migrated,
                removed,
            });
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn secret(value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }

    fn vault() -> (Arc<MemoryStore>, Vault) {
        let store = Arc::new(MemoryStore::new());
        (Arc::clone(&store), Vault::new(store))
    }

    #[test]
    fn a_provider_key_round_trips_under_its_short_name() {
        let (_, vault) = vault();
        assert_eq!(
            vault
                .set("openrouter", &secret("sk-or-1234567890"))
                .unwrap(),
            "ai_openrouter_api_key"
        );
        let key = vault.provider_key("openrouter").unwrap().unwrap();
        assert_eq!(key.expose_secret(), "sk-or-1234567890");
        assert!(vault.provider_key("deepseek").unwrap().is_none());
    }

    #[test]
    fn a_tampered_configuration_cannot_reach_an_infra_secret() {
        let (store, vault) = vault();
        vault
            .set("github_token", &secret("ghp_0123456789abcdef"))
            .unwrap();
        vault
            .set("neon_admin_key", &secret("napi_0123456789abcdef"))
            .unwrap();
        for hostile in [
            "github_token",
            "neon_admin_key",
            "../github_token",
            "github_token_api_key",
        ] {
            let found = vault.provider_key(hostile).ok().flatten();
            assert!(found.is_none(), "{hostile}");
        }
        // The store was only ever asked for entries inside `ai_*`.
        assert!(
            store
                .requested()
                .iter()
                .all(|entry| entry.starts_with("ai_")),
            "{:?}",
            store.requested()
        );
    }

    #[test]
    fn infra_and_db_secrets_can_be_stored_and_listed_but_have_no_read_path() {
        let (_, vault) = vault();
        vault
            .set("github_token", &secret("ghp_0123456789abcdef"))
            .unwrap();
        vault
            .set("openrouter", &secret("sk-or-1234567890"))
            .unwrap();
        let keys = vault.list().unwrap();
        let tiers: Vec<_> = keys.iter().map(|k| (k.name.as_str(), k.tier)).collect();
        assert_eq!(
            tiers,
            vec![
                ("ai_openrouter_api_key", Tier::Provider),
                ("github_token", Tier::Infra)
            ]
        );
        assert!(!format!("{keys:?}").contains("ghp_0123"));
    }

    #[test]
    fn unsafe_secrets_are_refused() {
        let (_, vault) = vault();
        for bad in ["", "   ", "line one\nline two", "tab\there", "esc\u{1b}[0m"] {
            assert!(vault.set("openrouter", &secret(bad)).is_err(), "{bad:?}");
        }
        assert!(
            vault
                .set("openrouter", &secret(&"a".repeat(MAX_SECRET_BYTES + 1)))
                .is_err()
        );
        assert!(vault.provider_key("openrouter").unwrap().is_none());
    }

    #[test]
    fn errors_never_contain_the_secret() {
        let (_, vault) = vault();
        let error = vault
            .set("Bad Name", &secret("sk-supersecret-value"))
            .unwrap_err()
            .to_string();
        assert!(!error.contains("supersecret"), "{error}");
    }

    #[test]
    fn remove_deletes_by_short_or_full_name() {
        let (_, vault) = vault();
        vault
            .set("openrouter", &secret("sk-or-1234567890"))
            .unwrap();
        assert_eq!(
            vault.remove("ai_openrouter_api_key").unwrap(),
            "ai_openrouter_api_key"
        );
        assert!(vault.provider_key("openrouter").unwrap().is_none());
        assert!(vault.remove("not a name").is_err());
    }

    #[test]
    fn cleanup_only_deletes_old_entries_that_have_a_kellu_twin() {
        let (store, vault) = vault();
        vault
            .set("openrouter", &secret("sk-or-1234567890"))
            .unwrap();
        store.add_legacy("ai_openrouter_api_key");
        store.add_legacy("github_token");

        let dry = vault.cleanup_legacy(false).unwrap();
        assert_eq!(dry.len(), 2);
        assert!(dry.iter().all(|entry| !entry.removed));

        let applied = vault.cleanup_legacy(true).unwrap();
        let removed: Vec<_> = applied
            .iter()
            .filter(|e| e.removed)
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(removed, vec!["ai_openrouter_api_key"]);
        let left: Vec<_> = vault
            .list()
            .unwrap()
            .into_iter()
            .filter(|k| k.legacy)
            .map(|k| k.name)
            .collect();
        assert_eq!(left, vec!["github_token".to_owned()]);
    }

    #[test]
    fn the_debug_output_is_opaque() {
        let (_, vault) = vault();
        assert_eq!(format!("{vault:?}"), "Vault { .. }");
    }
}
