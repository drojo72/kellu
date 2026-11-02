//! Provider-neutral data types.

use serde_json::{Map, Value};

/// The wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// OpenAI-compatible chat completions (OpenRouter, DeepSeek, xAI, local runtimes).
    OpenAiCompat,
    /// Google's Gemini API (also serves Gemma).
    Google,
}

impl ProviderKind {
    /// Every kind, sorted by configuration name.
    pub const ALL: [Self; 2] = [Self::Google, Self::OpenAiCompat];

    /// The name used in `config.toml`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompat => "openai_compat",
            Self::Google => "google",
        }
    }

    /// Parse a configuration name (exact match).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// One model provider.
///
/// `keyring_name` names a keyring entry, never the secret itself. `None` means the endpoint needs
/// no authentication (a local runtime).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderConfig {
    /// Name from the configuration (`[providers.<name>]`).
    pub name: String,
    /// Wire protocol.
    pub kind: ProviderKind,
    /// Endpoint base URL without a trailing slash.
    pub base_url: String,
    /// Model identifier sent to the provider.
    pub model: String,
    /// Per-request timeout in seconds.
    pub timeout_s: f64,
    /// Keyring name (`openrouter` resolves to the entry `ai_openrouter_api_key`).
    pub keyring_name: Option<String>,
}

/// A tool invocation requested by the model. `arguments` is already parsed JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Call identifier, echoed back in the tool result.
    pub id: String,
    /// Tool name, validated to `[A-Za-z0-9_.-]{1,64}`.
    pub name: String,
    /// Parsed arguments; always a JSON object.
    pub arguments: Map<String, Value>,
}

/// One conversation message. The variants make illegal combinations (a tool result without a
/// call id, tool calls on a user message) unrepresentable.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Instructions for the model.
    System(String),
    /// A user turn.
    User(String),
    /// A model turn, optionally requesting tool calls.
    Assistant {
        /// Text of the turn (may be empty when only tools are called).
        content: String,
        /// Tools the model asked to run.
        tool_calls: Vec<ToolCall>,
    },
    /// The result of running a tool.
    Tool {
        /// Id of the call this answers.
        call_id: String,
        /// Tool output.
        content: String,
    },
}

impl Message {
    /// A system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self::System(content.into())
    }

    /// A user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::User(content.into())
    }

    /// An assistant message without tool calls.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant {
            content: content.into(),
            tool_calls: Vec::new(),
        }
    }

    /// An assistant message that requests tool calls.
    pub fn assistant_with_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self::Assistant {
            content: content.into(),
            tool_calls,
        }
    }

    /// A tool result.
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool {
            call_id: call_id.into(),
            content: content.into(),
        }
    }
}

/// A tool the model may call. `parameters` is a JSON Schema object.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Tool name.
    pub name: String,
    /// What the tool does, for the model.
    pub description: String,
    /// JSON Schema of the arguments.
    pub parameters: Value,
}

/// A provider-neutral chat request.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// Conversation so far (never empty).
    pub messages: Vec<Message>,
    /// Tools the model may call.
    pub tools: Vec<ToolSpec>,
    /// Sampling temperature, if the caller sets one.
    pub temperature: Option<f64>,
    /// Maximum tokens to generate, if the caller sets one.
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    /// A request with the given messages, or `None` if there are none.
    #[must_use]
    pub fn new(messages: Vec<Message>) -> Option<Self> {
        if messages.is_empty() {
            return None;
        }
        Some(Self {
            messages,
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
        })
    }

    /// Offer tools to the model.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }

    /// Set the sampling temperature.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set the generation limit.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

/// Token counts, when the provider reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Tokens in the prompt.
    pub prompt_tokens: Option<u64>,
    /// Tokens generated.
    pub completion_tokens: Option<u64>,
}

/// A complete model answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    /// Answer text (thinking parts excluded).
    pub text: String,
    /// Name of the provider that answered.
    pub provider: String,
    /// Model reported by the provider.
    pub model: Option<String>,
    /// Tool calls requested by the model.
    pub tool_calls: Vec<ToolCall>,
    /// Why generation stopped, as the provider reports it.
    pub finish_reason: Option<String>,
    /// Token counts.
    pub usage: Option<Usage>,
}

/// One item of a streamed answer: text deltas, then a single final response.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A piece of answer text.
    Text(String),
    /// The complete response, always the last event of a successful stream.
    Done(Box<ChatResponse>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_name() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn all_is_sorted_by_name() {
        let names: Vec<_> = ProviderKind::ALL.iter().map(|k| k.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn unknown_or_differently_cased_names_are_rejected() {
        for name in ["", "openai", "Google", "OPENAI_COMPAT", "google "] {
            assert_eq!(ProviderKind::parse(name), None, "{name:?}");
        }
    }
}
