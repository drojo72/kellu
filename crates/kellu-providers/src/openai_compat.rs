//! Client for OpenAI-compatible chat-completions APIs (OpenRouter, DeepSeek, xAI, local).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use kellu_common::ProviderError;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use crate::base::{EventStream, KeyLookup, Provider, failed_stream};
use crate::common::{
    MAX_TOOL_CALLS, clean_field, make_tool_call, malformed, parse_usage, raise_if_error,
    resolve_key, str_field,
};
use crate::http::{AuthStyle, Credential, Limits, Transport};
use crate::stream::{Feed, Opening, ProviderStream, StreamParser};
use crate::types::{ChatRequest, ChatResponse, Message, ProviderConfig, ToolCall, Usage};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// `POST {base_url}/chat/completions` with Bearer authentication.
pub struct OpenAiCompatProvider {
    config: ProviderConfig,
    key_lookup: KeyLookup,
    transport: Transport,
}

fn tool_call_payload(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": Value::Object(call.arguments.clone()).to_string(),
        },
    })
}

fn message_payload(provider: &str, message: &Message) -> Result<Value, ProviderError> {
    Ok(match message {
        Message::System(content) => json!({"role": "system", "content": content}),
        Message::User(content) => json!({"role": "user", "content": content}),
        Message::Assistant {
            content,
            tool_calls,
        } => {
            let mut payload = json!({"role": "assistant", "content": content});
            if !tool_calls.is_empty() {
                payload["tool_calls"] =
                    Value::Array(tool_calls.iter().map(tool_call_payload).collect());
            }
            payload
        }
        Message::Tool { call_id, content } => {
            if call_id.is_empty() {
                return Err(
                    ProviderError::new(provider, "a tool message needs a call id").not_retryable(),
                );
            }
            json!({"role": "tool", "content": content, "tool_call_id": call_id})
        }
    })
}

/// Providers return a string, `null` (tool calls only) or a list of content parts.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

struct Prepared {
    url: String,
    headers: Vec<(&'static str, String)>,
    payload: Value,
    key: Option<Arc<SecretString>>,
}

impl OpenAiCompatProvider {
    /// A client for `config`; keys come from `key_lookup` on every request.
    #[must_use]
    pub fn new(config: ProviderConfig, key_lookup: KeyLookup) -> Self {
        Self::with_limits(config, key_lookup, Limits::default())
    }

    /// Like [`OpenAiCompatProvider::new`] with explicit transport limits (tests lower them).
    #[must_use]
    pub fn with_limits(config: ProviderConfig, key_lookup: KeyLookup, limits: Limits) -> Self {
        let timeout = Duration::try_from_secs_f64(config.timeout_s).unwrap_or(DEFAULT_TIMEOUT);
        Self {
            transport: Transport::new(timeout, limits),
            config,
            key_lookup,
        }
    }

    fn prepare(&self, request: &ChatRequest, stream: bool) -> Result<Prepared, ProviderError> {
        let key = resolve_key(&self.config, &self.key_lookup)?;
        let accept = if stream {
            "text/event-stream"
        } else {
            "application/json"
        };
        let messages = request
            .messages
            .iter()
            .map(|message| message_payload(&self.config.name, message))
            .collect::<Result<Vec<_>, _>>()?;
        let mut payload = json!({
            "model": self.config.model,
            "messages": messages,
            "stream": stream,
        });
        if !request.tools.is_empty() {
            payload["tools"] = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        },
                    })
                })
                .collect();
        }
        if let Some(temperature) = request.temperature {
            payload["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = request.max_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }
        Ok(Prepared {
            url: format!(
                "{}/chat/completions",
                self.config.base_url.trim_end_matches('/')
            ),
            headers: vec![
                ("content-type", "application/json".to_owned()),
                ("accept", accept.to_owned()),
            ],
            payload,
            key,
        })
    }

    fn credential(key: &Option<Arc<SecretString>>) -> Option<Credential> {
        key.as_ref()
            .map(|key| Credential::new(AuthStyle::Bearer, Arc::clone(key)))
    }

    fn tool_call(&self, index: usize, item: &Value) -> Result<ToolCall, ProviderError> {
        let Some(function) = item.get("function").and_then(Value::as_object) else {
            return Err(malformed(&self.config.name, "tool call without a function"));
        };
        make_tool_call(
            &self.config.name,
            index,
            item.get("id").and_then(Value::as_str),
            str_field(function, "name"),
            function.get("arguments"),
        )
    }
}

impl Provider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let name = self.config.name.as_str();
        let prepared = self.prepare(request, false)?;
        let headers: Vec<(&str, &str)> = prepared
            .headers
            .iter()
            .map(|(n, v)| (*n, v.as_str()))
            .collect();
        let credential = Self::credential(&prepared.key);
        let data = self.transport.post_json(
            name,
            &prepared.url,
            &headers,
            credential.as_ref(),
            &prepared.payload,
        )?;
        let key = prepared.key.as_ref().map(|key| key.expose_secret());
        raise_if_error(name, &data, key)?;
        let Some(choice) = data
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(Value::as_object)
        else {
            return Err(malformed(name, "no choices"));
        };
        let Some(message) = choice.get("message").and_then(Value::as_object) else {
            return Err(malformed(name, "choice without a message"));
        };
        let raw_calls: &[Value] = match message.get("tool_calls") {
            None | Some(Value::Null) => &[],
            Some(Value::Array(calls)) => calls,
            Some(_) => return Err(malformed(name, "tool_calls is not a list")),
        };
        if raw_calls.len() > MAX_TOOL_CALLS {
            return Err(malformed(name, "too many tool calls"));
        }
        let tool_calls = raw_calls
            .iter()
            .enumerate()
            .map(|(index, item)| self.tool_call(index, item))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ChatResponse {
            text: content_text(message.get("content")),
            provider: name.to_owned(),
            model: str_field(&data, "model").map(clean_field),
            tool_calls,
            finish_reason: str_field(choice, "finish_reason").map(clean_field),
            usage: parse_usage(data.get("usage"), "prompt_tokens", "completion_tokens"),
        })
    }

    fn stream(&self, request: &ChatRequest) -> EventStream {
        let prepared = match self.prepare(request, true) {
            Ok(prepared) => prepared,
            Err(error) => return failed_stream(error),
        };
        let parser = OpenAiParser {
            provider: self.config.name.clone(),
            key: prepared.key.clone(),
            ..OpenAiParser::default()
        };
        Box::new(ProviderStream::new(
            Opening {
                transport: self.transport.clone(),
                provider: self.config.name.clone(),
                url: prepared.url,
                headers: prepared.headers,
                credential: Self::credential(&prepared.key),
                payload: prepared.payload,
            },
            parser,
        ))
    }
}

#[derive(Default)]
struct PartialCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Default)]
struct OpenAiParser {
    provider: String,
    key: Option<Arc<SecretString>>,
    text: String,
    calls: BTreeMap<usize, PartialCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    model: Option<String>,
    done: bool,
}

impl OpenAiParser {
    fn accumulate(&mut self, item: &Value) -> Result<(), ProviderError> {
        let Some(item) = item.as_object() else {
            return Err(malformed(
                &self.provider,
                "tool call delta is not an object",
            ));
        };
        let index = match item.get("index") {
            None => Some(0),
            Some(value) => value.as_u64(),
        };
        let Some(index) = index
            .and_then(|index| usize::try_from(index).ok())
            .filter(|&index| index < MAX_TOOL_CALLS)
        else {
            return Err(malformed(
                &self.provider,
                "tool call delta with an invalid index",
            ));
        };
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = str_field(item, "id") {
            entry.id = Some(id.to_owned());
        }
        if let Some(function) = item.get("function").and_then(Value::as_object) {
            if let Some(name) = str_field(function, "name") {
                entry.name = Some(name.to_owned());
            }
            if let Some(arguments) = str_field(function, "arguments") {
                entry.arguments.push_str(arguments);
            }
        }
        Ok(())
    }
}

impl StreamParser for OpenAiParser {
    fn feed(&mut self, data: &str) -> Result<Feed, ProviderError> {
        if data.trim() == "[DONE]" {
            self.done = true;
            return Ok(Feed::Done);
        }
        let Ok(Value::Object(chunk)) = serde_json::from_str::<Value>(data) else {
            let detail = if serde_json::from_str::<Value>(data).is_ok() {
                "stream event is not an object"
            } else {
                "stream event is not JSON"
            };
            return Err(malformed(&self.provider, detail));
        };
        raise_if_error(
            &self.provider,
            &chunk,
            self.key.as_ref().map(|key| key.expose_secret()),
        )?;
        if let Some(model) = str_field(&chunk, "model") {
            self.model = Some(clean_field(model));
        }
        self.usage =
            parse_usage(chunk.get("usage"), "prompt_tokens", "completion_tokens").or(self.usage);
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(Value::as_object)
        else {
            return Ok(Feed::Skip);
        };
        let mut piece = String::new();
        if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
            piece = content_text(delta.get("content"));
            match delta.get("tool_calls") {
                None | Some(Value::Null) => {}
                Some(Value::Array(items)) => {
                    for item in items {
                        self.accumulate(item)?;
                    }
                }
                Some(_) => {
                    return Err(malformed(
                        &self.provider,
                        "tool call delta is not an object",
                    ));
                }
            }
        }
        if let Some(reason) = str_field(choice, "finish_reason") {
            self.finish_reason = Some(clean_field(reason));
        }
        if piece.is_empty() {
            return Ok(Feed::Skip);
        }
        self.text.push_str(&piece);
        Ok(Feed::Text(piece))
    }

    fn finish(self) -> Result<ChatResponse, ProviderError> {
        if !self.done && self.finish_reason.is_none() {
            return Err(ProviderError::new(
                &self.provider,
                "stream ended unexpectedly",
            ));
        }
        let tool_calls = self
            .calls
            .iter()
            .map(|(&index, call)| {
                make_tool_call(
                    &self.provider,
                    index,
                    call.id.as_deref(),
                    call.name.as_deref(),
                    Some(&Value::String(call.arguments.clone())),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ChatResponse {
            text: self.text,
            provider: self.provider,
            model: self.model,
            tool_calls,
            finish_reason: self.finish_reason,
            usage: self.usage,
        })
    }
}
