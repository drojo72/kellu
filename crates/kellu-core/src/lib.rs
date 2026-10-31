//! Kellu core: configuration, logging and sessions (the router and the cascade follow in M2 and
//! later). Nothing here runs at import time, and constructing a value never touches the
//! filesystem unless the constructor's documentation says so.

pub mod config;
pub mod log;
pub mod session;
