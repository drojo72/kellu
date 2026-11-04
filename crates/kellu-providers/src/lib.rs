//! Model providers: provider-neutral types, a hardened HTTP transport, an OpenAI-compatible
//! client and a Google (Gemini and Gemma) client.
//!
//! Layering: this crate depends only on `kellu-common`. Keys never live here beyond one request:
//! callers pass a [`base::KeyLookup`], and keys are held in `secrecy` types.

pub mod base;
pub mod google;
pub mod http;
pub mod openai_compat;
pub mod registry;
pub mod types;

mod common;
mod stream;

pub use base::{EventStream, KeyLookup, Provider};
pub use registry::{build_provider, build_provider_capped};
pub use types::{
    ChatRequest, ChatResponse, Message, ProviderConfig, ProviderKind, StreamEvent, ToolCall,
    ToolSpec, Usage,
};
