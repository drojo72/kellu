//! Kellu's tools: what the model may ask for, and the rules every request goes through.
//!
//! * Every tool declares exactly one [`Risk`](kellu_security::Risk); the registry enforces the
//!   requirement for that risk (`kellu_security::requirement`) before the tool runs.
//! * Tools never write the real project. A tool that wants a change returns a
//!   [`Proposal`]; only [`apply`] writes, after the user approved it, and only what was approved,
//!   only if the files still have the hash the user saw.
//! * Commands run in the sandbox on a staging copy. Git and `gh` are the exception: they run on
//!   the host with a hardened environment, and anything beyond reading needs confirmation.
//!
//! Nothing here runs at import time or touches the filesystem, the network or a process until a
//! caller asks for it. Every refusal fails closed, and no error repeats a value it was given.

pub mod apply;
pub mod args;
pub mod fs_tools;
pub mod git;
pub mod host;
pub mod proposal;
pub mod registry;
pub mod shell;
pub mod spec;

pub use apply::{Applied, Approval, Approved, Choice, Report, Session, apply, approve};
pub use proposal::{Hunk, Item, Operation, Proposal};
pub use registry::{
    Approver, CallResult, ConfirmRequest, Confirmation, Registry, default_registry,
};
pub use spec::{Context, Tool, ToolError, ToolOutput, ToolSpec};
