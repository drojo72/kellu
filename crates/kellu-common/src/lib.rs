//! Shared foundations: identity constants, errors, XDG paths and private files.
//!
//! This is the lowest layer: it depends on no other Kellu crate. Nothing here runs at import
//! time or touches the filesystem unless a caller explicitly asks for it.

#[cfg(not(unix))]
compile_error!("kellu-common supports Unix-like systems only for now (Linux first).");

pub mod app;
pub mod error;
pub mod paths;

pub use error::{KelluError, ProviderError, Result};
