//! The Kate LSP server (0.1.0: hover and completion), over standard input and output.
//!
//! Standard output carries the protocol and nothing else. Everything the editor sends is untrusted:
//! messages are size-capped, document paths are checked against the project scope and the deny
//! globs, and text reaches a model only through `kellu_core::assist`.

pub mod server;
pub mod text;
pub mod transport;
pub mod uri;

pub use server::{Features, Flow, Server};
