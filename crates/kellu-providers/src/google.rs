//! Client for the Google Gemini API (`generateContent`), used for Gemini and Gemma models.
//!
//! The API key goes in the `x-goog-api-key` header, never in the URL. Gemma models do not accept
//! a system instruction, so system messages are folded into the first user turn for them.

use std::sync::Arc;
use std::time::Duration;

use kellu_common::ProviderError;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use crate::base::{EventStream, KeyLookup, Provider, failed_stream};
use crate::common::{clean_field, malformed, parse_usage, raise_if_error, resolve_key, str_field};
use crate::http::{AuthStyle, Credential, Limits, Transport, scrub};
use crate::stream::{Feed, Opening, ProviderStream, StreamParser};
use crate::types::{ChatRequest, ChatResponse, Message, ProviderConfig, Usage};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

fn strip_models_prefix(model: &str) -> &str {
    model.strip_prefix("models/").unwrap_or(model)
}

fn inline_system(model: &str) -> bool {
    strip_models_prefix(&model.to_ascii_lowercase()).starts_with("gemma")
}

/// Percent-encode one URL path segment (everything except unreserved characters).
fn encode_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The answer text of one candidate, without "thought" parts.
fn text_of(candidate: &Map<String, Value>) -> String {
    let parts = candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array);
    parts
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|part| !part.get("thought").is_some_and(is_truthy))
        .filter_map(|part| str_field(part, "text"))
        .collect()
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

fn usage_of(data: &Map<String, Value>) -> Option<Usage> {
    parse_usage(
        data.get("usageMetadata"),
        "promptTokenCount",
        "candidatesTokenCount",
    )
}

fn raise_if_blocked(provider: &str, data: &Map<String, Value>) -> Result<(), ProviderError> {
    let reason = data
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
        .filter(|reason| is_truthy(reason));
    match reason {
        Some(reason) => {
            let reason = reason
                .as_str()
                .map_or_else(|| reason.to_string(), str::to_owned);
            Err(ProviderError::new(
                provider,
                format!("prompt blocked by the provider ({})", scrub(&reason, None)),
            )
            .not_retryable())
        }
        None => Ok(()),
    }
}

/// Text chat over `models/{model}:generateContent`. Tool calling is not wired yet.
pub struct GoogleProvider {
    config: ProviderConfig,
    key_lookup: KeyLookup,
    transport: Transport,
}

struct Prepared {
    headers: Vec<(&'static str, String)>,
    payload: Value,
    key: Option<Arc<SecretString>>,
}

impl GoogleProvider {
    /// A client for `config`; keys come from `key_lookup` on every request.
    #[must_use]
    pub fn new(config: ProviderConfig, key_lookup: KeyLookup) -> Self {
        Self::with_limits(config, key_lookup, Limits::default())
    }

    /// Like [`GoogleProvider::new`] with explicit transport limits (tests lower them).
    #[must_use]
    pub fn with_limits(config: ProviderConfig, key_lookup: KeyLookup, limits: Limits) -> Self {
        let timeout = Duration::try_from_secs_f64(config.timeout_s).unwrap_or(DEFAULT_TIMEOUT);
        Self {
            transport: Transport::new(timeout, limits),
            config,
            key_lookup,
        }
    }

    fn fail(&self, reason: &str) -> ProviderError {
        ProviderError::new(&self.config.name, reason).not_retryable()
    }

    fn prepare(&self, request: &ChatRequest) -> Result<Prepared, ProviderError> {
        if !request.tools.is_empty() {
            return Err(self.fail("tool calling is not supported yet"));
        }
        let key = resolve_key(&self.config, &self.key_lookup)?;

        let mut system: Vec<&str> = Vec::new();
        let mut turns: Vec<(&str, String)> = Vec::new();
        for message in &request.messages {
            match message {
                Message::System(content) => {
                    if !content.is_empty() {
                        system.push(content);
                    }
                }
                Message::User(content) => turns.push(("user", content.clone())),
                Message::Assistant {
                    content,
                    tool_calls,
                } => {
                    if !tool_calls.is_empty() {
                        return Err(self.fail("tool messages are not supported yet"));
                    }
                    turns.push(("model", content.clone()));
                }
                Message::Tool { .. } => {
                    return Err(self.fail("tool messages are not supported yet"));
                }
            }
        }
        if turns.is_empty() {
            return Err(self.fail("the request has no user or assistant turns"));
        }

        let mut system_instruction = None;
        if !system.is_empty() {
            let text = system.join("\n\n");
            if inline_system(&self.config.model) {
                match turns.first_mut() {
                    Some((role, first)) if *role == "user" => *first = format!("{text}\n\n{first}"),
                    _ => turns.insert(0, ("user", text)),
                }
            } else {
                system_instruction = Some(json!({"parts": [{"text": text}]}));
            }
        }

        let contents: Vec<Value> = turns
            .into_iter()
            .map(|(role, text)| json!({"role": role, "parts": [{"text": text}]}))
            .collect();
        let mut payload = json!({"contents": contents});
        if let Some(instruction) = system_instruction {
            payload["systemInstruction"] = instruction;
        }
        let mut generation = Map::new();
        if let Some(temperature) = request.temperature {
            generation.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            generation.insert("maxOutputTokens".to_owned(), json!(max_tokens));
        }
        if !generation.is_empty() {
            payload["generationConfig"] = Value::Object(generation);
        }
        Ok(Prepared {
            headers: vec![("content-type", "application/json".to_owned())],
            payload,
            key,
        })
    }

    fn url(&self, method: &str, query: &str) -> String {
        format!(
            "{}/models/{}:{method}{query}",
            self.config.base_url.trim_end_matches('/'),
            encode_segment(strip_models_prefix(&self.config.model)),
        )
    }

    fn credential(key: &Option<Arc<SecretString>>) -> Option<Credential> {
        key.as_ref()
            .map(|key| Credential::new(AuthStyle::GoogleKey, Arc::clone(key)))
    }
}

impl Provider for GoogleProvider {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_tools(&self) -> bool {
        false
    }

    fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let name = self.config.name.as_str();
        let prepared = self.prepare(request)?;
        let headers: Vec<(&str, &str)> = prepared
            .headers
            .iter()
            .map(|(n, v)| (*n, v.as_str()))
            .collect();
        let credential = Self::credential(&prepared.key);
        let data = self.transport.post_json(
            name,
            &self.url("generateContent", ""),
            &headers,
            credential.as_ref(),
            &prepared.payload,
        )?;
        raise_if_error(
            name,
            &data,
            prepared.key.as_ref().map(|key| key.expose_secret()),
        )?;
        let Some(candidate) = data
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
        else {
            raise_if_blocked(name, &data)?;
            return Err(malformed(name, "no candidates"));
        };
        let Some(candidate) = candidate.as_object() else {
            return Err(malformed(name, "candidate is not an object"));
        };
        Ok(ChatResponse {
            text: text_of(candidate),
            provider: name.to_owned(),
            model: str_field(&data, "modelVersion").map(clean_field),
            tool_calls: Vec::new(),
            finish_reason: str_field(candidate, "finishReason").map(clean_field),
            usage: usage_of(&data),
        })
    }

    fn stream(&self, request: &ChatRequest) -> EventStream {
        let prepared = match self.prepare(request) {
            Ok(prepared) => prepared,
            Err(error) => return failed_stream(error),
        };
        let mut headers = prepared.headers;
        headers.push(("accept", "text/event-stream".to_owned()));
        let parser = GoogleParser {
            provider: self.config.name.clone(),
            key: prepared.key.clone(),
            text: String::new(),
            finish_reason: None,
            usage: None,
            model: None,
            received: false,
        };
        Box::new(ProviderStream::new(
            Opening {
                transport: self.transport.clone(),
                provider: self.config.name.clone(),
                url: self.url("streamGenerateContent", "?alt=sse"),
                headers,
                credential: Self::credential(&prepared.key),
                payload: prepared.payload,
            },
            parser,
        ))
    }
}

struct GoogleParser {
    provider: String,
    key: Option<Arc<SecretString>>,
    text: String,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    model: Option<String>,
    received: bool,
}

impl StreamParser for GoogleParser {
    fn feed(&mut self, data: &str) -> Result<Feed, ProviderError> {
        let chunk = match serde_json::from_str::<Value>(data) {
            Ok(Value::Object(chunk)) => chunk,
            Ok(_) => return Err(malformed(&self.provider, "stream event is not an object")),
            Err(_) => return Err(malformed(&self.provider, "stream event is not JSON")),
        };
        raise_if_error(
            &self.provider,
            &chunk,
            self.key.as_ref().map(|key| key.expose_secret()),
        )?;
        self.received = true;
        if let Some(model) = str_field(&chunk, "modelVersion") {
            self.model = Some(clean_field(model));
        }
        self.usage = usage_of(&chunk).or(self.usage);
        let Some(candidate) = chunk
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
        else {
            raise_if_blocked(&self.provider, &chunk)?;
            return Ok(Feed::Skip);
        };
        let Some(candidate) = candidate.as_object() else {
            return Err(malformed(&self.provider, "candidate is not an object"));
        };
        if let Some(reason) = str_field(candidate, "finishReason") {
            self.finish_reason = Some(clean_field(reason));
        }
        let piece = text_of(candidate);
        if piece.is_empty() {
            return Ok(Feed::Skip);
        }
        self.text.push_str(&piece);
        Ok(Feed::Text(piece))
    }

    fn finish(self) -> Result<ChatResponse, ProviderError> {
        if !self.received {
            return Err(ProviderError::new(
                &self.provider,
                "stream ended without data",
            ));
        }
        Ok(ChatResponse {
            text: self.text,
            provider: self.provider,
            model: self.model,
            tool_calls: Vec::new(),
            finish_reason: self.finish_reason,
            usage: self.usage,
        })
    }
}
