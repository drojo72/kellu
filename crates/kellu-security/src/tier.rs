//! Secret tiers and entry names.
//!
//! Tiers are **not stored** in the keyring: they are deduced from the entry name, so a tampered
//! keyring cannot promote a secret. A name that matches nothing is `infra`, so unknown secrets
//! fail closed (the assistant cannot reach them).

use kellu_common::{KelluError, Result};

/// How reachable a secret is for the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Model provider API keys: used by the core in the normal flow.
    Provider,
    /// Cloud, DNS and deployment tokens: not reachable unless explicitly enabled.
    Infra,
    /// Database passwords and admin keys: same as `Infra`.
    Db,
}

impl Tier {
    /// Lowercase name, as shown by `kellu keys list`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Infra => "infra",
            Self::Db => "db",
        }
    }
}

const PROVIDER_PREFIXES: &[&str] = &["ai_"];
const INFRA_PREFIXES: &[&str] = &["github_", "cloudflare_", "render_", "desec_", "duckdns_"];
const DB_PREFIXES: &[&str] = &["neon_", "supabase_", "mysql_"];

/// Longest accepted entry name.
pub const MAX_ENTRY_LEN: usize = 64;

/// The tier of keyring entry `entry`, deduced from its prefix. Unknown names are `Infra`.
#[must_use]
pub fn tier_of(entry: &str) -> Tier {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| entry.starts_with(p));
    if starts(PROVIDER_PREFIXES) {
        Tier::Provider
    } else if starts(DB_PREFIXES) {
        Tier::Db
    } else {
        // The known infra prefixes and, deliberately, every unknown name.
        Tier::Infra
    }
}

/// Whether `entry` begins with one of the known tier prefixes (so it is a full entry name and
/// not the short name of a provider).
#[must_use]
pub fn has_known_prefix(entry: &str) -> bool {
    [PROVIDER_PREFIXES, INFRA_PREFIXES, DB_PREFIXES]
        .iter()
        .flat_map(|list| list.iter())
        .any(|p| entry.starts_with(p))
}

/// Whether `name` is a syntactically valid entry name: `[a-z][a-z0-9_-]{0,63}`.
///
/// Names end up in `argv` and in labels, so the alphabet is deliberately small.
#[must_use]
pub fn is_valid_entry_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= MAX_ENTRY_LEN
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The keyring entry for a provider's `keyring_name`: `ai_<keyring_name>_api_key`.
///
/// The result is always in the `ai_*` space (tier `provider`), whatever the input, so a
/// tampered configuration cannot make Kellu read an `infra` or `db` secret and send it to
/// another endpoint.
///
/// # Errors
/// The name is not a valid entry-name fragment.
pub fn provider_entry(keyring_name: &str) -> Result<String> {
    let entry = format!("ai_{keyring_name}_api_key");
    if !is_valid_entry_name(&entry) || !is_valid_entry_name(keyring_name) {
        return Err(KelluError::config(
            "keyring_name: expected a short lowercase name (a-z, 0-9, _ or -)",
        ));
    }
    debug_assert_eq!(tier_of(&entry), Tier::Provider);
    Ok(entry)
}

/// Resolve what the user typed into a full entry name.
///
/// A name with a known tier prefix (`github_token`, `ai_x_api_key`) is a full entry name;
/// anything else is the short name of a provider (`openrouter` → `ai_openrouter_api_key`).
///
/// # Errors
/// The name is not valid.
pub fn resolve_entry(input: &str) -> Result<String> {
    if has_known_prefix(input) {
        if is_valid_entry_name(input) {
            Ok(input.to_owned())
        } else {
            Err(KelluError::config(
                "invalid key name (use a-z, 0-9, _ or -; at most 64 characters)",
            ))
        }
    } else {
        provider_entry(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_follow_the_prefix() {
        assert_eq!(tier_of("ai_openrouter_api_key"), Tier::Provider);
        for name in [
            "github_token",
            "cloudflare_api_token",
            "render_api_key",
            "desec_token",
        ] {
            assert_eq!(tier_of(name), Tier::Infra, "{name}");
        }
        for name in ["neon_admin_key", "supabase_admin_key", "mysql_root"] {
            assert_eq!(tier_of(name), Tier::Db, "{name}");
        }
    }

    #[test]
    fn unknown_names_fail_closed_as_infra() {
        assert_eq!(tier_of("test_key"), Tier::Infra);
        assert_eq!(tier_of("AI_upper"), Tier::Infra);
        assert_eq!(tier_of(""), Tier::Infra);
        assert_eq!(tier_of("gemma_direct_api_key"), Tier::Infra);
    }

    #[test]
    fn entry_names_have_a_small_alphabet() {
        assert!(is_valid_entry_name("ai_openrouter_api_key"));
        assert!(is_valid_entry_name("a-b_c9"));
        for bad in [
            "",
            "Ai_x",
            "1ai",
            "ai x",
            "ai/x",
            "ai\nx",
            "ai=x",
            "-x",
            "é",
            &"a".repeat(65),
        ] {
            assert!(!is_valid_entry_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_provider_name_always_maps_into_the_ai_space() {
        assert_eq!(
            provider_entry("openrouter").unwrap(),
            "ai_openrouter_api_key"
        );
        // A name that looks like another tier is still inside `ai_*`, never an infra entry.
        let entry = provider_entry("github_token").unwrap();
        assert_eq!(entry, "ai_github_token_api_key");
        assert_eq!(tier_of(&entry), Tier::Provider);
    }

    #[test]
    fn hostile_provider_names_are_rejected() {
        for bad in [
            "",
            "A",
            "x y",
            "x\0y",
            "../x",
            "x/y",
            "x\ny",
            &"a".repeat(80),
        ] {
            assert!(provider_entry(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn short_names_and_full_names_are_told_apart() {
        assert_eq!(
            resolve_entry("openrouter").unwrap(),
            "ai_openrouter_api_key"
        );
        assert_eq!(
            resolve_entry("ai_openrouter_api_key").unwrap(),
            "ai_openrouter_api_key"
        );
        assert_eq!(resolve_entry("github_token").unwrap(), "github_token");
        assert!(resolve_entry("github_Token").is_err());
        assert!(resolve_entry("github_x y").is_err());
    }
}
