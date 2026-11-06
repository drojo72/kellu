//! Kellu's sandbox: bubblewrap, a policy derived from a [`kellu_security::Grant`], a staging copy
//! of the files the grant allows, and resource limits.
//!
//! The real project is never mounted and never written by anything in this crate. A tool runs on a
//! staging copy; [`Staging::changes`] reports what it did, and the core decides what becomes a
//! proposal. Nothing here runs at import time, and nothing touches the filesystem or starts a
//! process unless a caller asks for it. Every refusal fails closed.

pub mod bwrap;
pub mod limits;
pub mod policy;
pub mod probe;
pub mod run;
pub mod staging;

pub use bwrap::{BwrapVersion, FIRST_PATCHED};
pub use limits::Limits;
pub use policy::{Access, Policy, Profile, SystemEntry, SystemLayout};
pub use probe::{Probe, probe};
pub use run::{Outcome, Output, Sandbox};
pub use staging::{Change, SkipReason, Skipped, Staging, StagingLimits};
