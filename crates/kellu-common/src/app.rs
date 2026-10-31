//! Single source of truth for the application's identity.
//!
//! Everything that depends on the project name (paths, environment prefix, keyring schema)
//! derives from here, so a rename touches one file.

/// Crate and binary name, also the directory name under the XDG base directories.
pub const NAME: &str = "kellu";
/// Name shown to people.
pub const DISPLAY_NAME: &str = "Kellu";
/// Prefix of every environment variable Kellu reads.
pub const ENV_PREFIX: &str = "KELLU_";
/// Version, single-sourced from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Keyring entries carry one attribute named like the app, whose value is `<service>_<key>`
/// (for example `kellu=ai_openrouter_api_key`), plus the label `KELLU: <name>`.
pub const KEYRING_ATTRIBUTE: &str = "kellu";
/// Attribute used by the earlier prototype; read only by the migration command (M3).
pub const LEGACY_KEYRING_ATTRIBUTE: &str = "kai";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_consistent() {
        assert_eq!(NAME, "kellu");
        assert_eq!(ENV_PREFIX, "KELLU_");
        assert_eq!(KEYRING_ATTRIBUTE, NAME);
        assert_ne!(LEGACY_KEYRING_ATTRIBUTE, KEYRING_ATTRIBUTE);
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }
}
