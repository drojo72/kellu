//! Kellu core: configuration, logging, sessions and the provider router (the cascade follows in
//! later milestones). Nothing here runs at import time, and constructing a value never touches the
//! filesystem unless the constructor's documentation says so.

pub mod assist;
pub mod config;
pub mod log;
pub mod router;
pub mod session;
