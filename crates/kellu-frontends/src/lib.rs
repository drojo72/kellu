//! Kellu frontends: thin adapters between a user interface and the core. They decide nothing about
//! policy; every text that leaves the machine goes through `kellu_core::assist`.
//!
//! Nothing here runs at import time, and constructing a value never touches the filesystem.

pub mod lsp;
pub mod repl;
pub mod theme;
