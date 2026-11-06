//! Kellu's security layer: keyring vault, scrubbed environments, redaction, deny globs, project
//! scope, grants, risk-based permissions and the audit log.
//!
//! It depends only on `kellu-common`. Nothing here runs at import time or touches the
//! filesystem, the keyring or the network unless a caller explicitly asks for it.

pub mod audit;
pub mod deny;
pub mod env;
pub mod grant;
pub mod hash;
pub mod permission;
pub mod redact;
pub mod scope;
pub mod store;
pub mod tier;
pub mod vault;

pub use audit::{AuditEvent, AuditLog, AuditRecord};
pub use deny::DenyGlobs;
pub use grant::Grant;
pub use permission::{Requirement, Risk, requirement};
pub use redact::PatternRedactor;
pub use scope::{Scope, openat2_available};
pub use store::{EntryInfo, MemoryStore, SecretStore, SecretToolStore};
pub use tier::Tier;
pub use vault::{KeyInfo, LegacyEntry, Vault};
