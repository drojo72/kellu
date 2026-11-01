//! Helpers shared by the provider clients.

use std::sync::Arc;

use kellu_common::ProviderError;
use kellu_common::text::strip_control_chars;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};

use crate::base::KeyLookup;
use crate::http::{retryable_status, scrub};
use crate::types::{ProviderConfig, ToolCall, Usage};

/// Most tool calls accepted in one answer.
pub(crate) const MAX_TOOL_CALLS: usize = 32;
const MAX_TOOL_NAME_CHARS: usize = 64;
const MAX_TOOL_ID_CHARS: usize = 128;
const MAX_FIELD_CHARS: usize = 200;

/// A response that does not look like what the API promises (not worth retrying).
pub(crate) fn malformed(provider: &str, detail: &str) -> ProviderError {
    ProviderError::new(provider, format!("malformed response: {detail}"))
        .with_status(200)
        .not_retryable()
}

/// The API key for `config`; `Ok(None)` for providers without authentication.
///
/// # Errors
/// A non-retryable error if a key is required but missing, or unsafe to put in a header.
pub(crate) fn resolve_key(
    config: &ProviderConfig,
    lookup: &KeyLookup,
) -> Result<Option<Arc<SecretString>>, ProviderError> {
    let Some(name) = config.keyring_name.as_deref() else {
        return Ok(None);
    };
    let found = lookup(name);
    let key = found
        .as_ref()
        .map_or("", |secret| secret.expose_secret().trim());
    if key.is_empty() {
        return Err(ProviderError::new(
            &config.name,
            format!("no API key for '{name}'; run `kellu keys set {name}`"),
        )
        .not_retryable());
    }
    if !key.chars().all(|c| c.is_ascii_graphic()) {
        return Err(
            ProviderError::new(&config.name, "the stored API key is malformed").not_retryable(),
        );
    }
    Ok(Some(Arc::new(SecretString::from(key.to_owned()))))
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

/// Some gateways answer HTTP 200 with an `error` object, also inside streams.
pub(crate) fn raise_if_error(
    provider: &str,
    data: &Map<String, Value>,
    key: Option<&str>,
) -> Result<(), ProviderError> {
    let Some(error) = data.get("error").filter(|value| is_truthy(value)) else {
        return Ok(());
    };
    let (message, code) = match error {
        Value::Object(fields) => (fields.get("message"), fields.get("code")),
        other => (Some(other), None),
    };
    let message = match message {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "unknown error".to_owned(),
    };
    let status = code
        .and_then(Value::as_u64)
        .and_then(|code| u16::try_from(code).ok())
        .unwrap_or(0);
    Err(ProviderError::new(provider, scrub(&message, key))
        .with_status(status)
        .retryable(status == 0 || retryable_status(status)))
}

/// Turn tool-call arguments (a JSON string or an object) into a map.
pub(crate) fn parse_arguments(
    provider: &str,
    raw: Option<&Value>,
) -> Result<Map<String, Value>, ProviderError> {
    match raw {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(fields)) => Ok(fields.clone()),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(Map::new()),
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(fields)) => Ok(fields),
            Ok(_) => Err(malformed(
                provider,
                "tool call arguments must be a JSON object",
            )),
            Err(_) => Err(malformed(
                provider,
                "tool call arguments are not valid JSON",
            )),
        },
        Some(_) => Err(malformed(
            provider,
            "tool call arguments have an unexpected type",
        )),
    }
}

fn valid_tool_name(name: &str) -> bool {
    (1..=MAX_TOOL_NAME_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// A provider-supplied label (model, finish reason) made safe to show: no control or format
/// characters, bounded.
pub(crate) fn clean_field(text: &str) -> String {
    strip_control_chars(text)
        .chars()
        .filter(|c| !matches!(c, '\n' | '\t'))
        .take(MAX_FIELD_CHARS)
        .collect()
}

fn valid_tool_id(id: &str) -> bool {
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b':'))
}

/// Validate and build a [`ToolCall`] from untrusted provider data.
///
/// `index` is the call's position in the answer. A missing or unsafe id is replaced by
/// `call_<index>_<name>`, which is unique within the answer, so results can be matched to calls.
pub(crate) fn make_tool_call(
    provider: &str,
    index: usize,
    id: Option<&str>,
    name: Option<&str>,
    arguments: Option<&Value>,
) -> Result<ToolCall, ProviderError> {
    let Some(name) = name.filter(|name| valid_tool_name(name)) else {
        return Err(malformed(provider, "tool call with an invalid name"));
    };
    let id = match id {
        Some(id) if !id.is_empty() && valid_tool_id(id) => {
            id.chars().take(MAX_TOOL_ID_CHARS).collect()
        }
        _ => format!("call_{index}_{name}"),
    };
    Ok(ToolCall {
        id,
        name: name.to_owned(),
        arguments: parse_arguments(provider, arguments)?,
    })
}

/// Read token counts from a provider usage object.
pub(crate) fn parse_usage(
    raw: Option<&Value>,
    prompt_key: &str,
    completion_key: &str,
) -> Option<Usage> {
    let Some(Value::Object(fields)) = raw else {
        return None;
    };
    Some(Usage {
        prompt_tokens: fields.get(prompt_key).and_then(Value::as_u64),
        completion_tokens: fields.get(completion_key).and_then(Value::as_u64),
    })
}

/// A string field of a JSON object, if present and a string.
pub(crate) fn str_field<'a>(data: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    data.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::ProviderKind;

    fn config(keyring_name: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            name: "p".to_owned(),
            kind: ProviderKind::OpenAiCompat,
            base_url: "https://example.com".to_owned(),
            model: "m".to_owned(),
            timeout_s: 5.0,
            keyring_name: keyring_name.map(str::to_owned),
        }
    }

    fn lookup(value: Option<&'static str>) -> KeyLookup {
        Arc::new(move |_| value.map(|v| SecretString::from(v.to_owned())))
    }

    #[test]
    fn a_trailing_newline_is_trimmed_from_the_key() {
        let key = resolve_key(&config(Some("p")), &lookup(Some("sk-abc123\n"))).unwrap();
        assert_eq!(key.unwrap().expose_secret(), "sk-abc123");
    }

    #[test]
    fn missing_empty_or_blank_keys_are_refused_with_the_command_to_fix_it() {
        for value in [None, Some(""), Some("  \n")] {
            let error = resolve_key(&config(Some("p")), &lookup(value)).unwrap_err();
            assert!(!error.retryable);
            assert!(
                error.reason.contains("kellu keys set p"),
                "{}",
                error.reason
            );
        }
    }

    #[test]
    fn keys_that_could_inject_headers_are_refused() {
        for value in ["a b", "ab\r\nX-Evil: 1", "ab\tcd", "ab\u{7f}cd", "clavé"] {
            let error = resolve_key(&config(Some("p")), &lookup(Some(value))).unwrap_err();
            assert_eq!(error.reason, "the stored API key is malformed");
            assert!(!error.retryable);
        }
    }

    #[test]
    fn providers_without_a_keyring_name_need_no_key() {
        assert!(resolve_key(&config(None), &lookup(None)).unwrap().is_none());
    }

    #[test]
    fn tool_names_are_validated() {
        for name in ["read_file", "a.b-c", &"x".repeat(64)] {
            assert!(
                make_tool_call("p", 0, Some("1"), Some(name), None).is_ok(),
                "{name}"
            );
        }
        for name in [
            "",
            "has space",
            "semi;colon",
            "sl/ash",
            "ñ",
            &"x".repeat(65),
        ] {
            assert!(
                make_tool_call("p", 0, Some("1"), Some(name), None).is_err(),
                "{name:?}"
            );
        }
        assert!(make_tool_call("p", 0, Some("1"), None, None).is_err());
    }

    #[test]
    fn arguments_must_be_a_json_object() {
        let ok = parse_arguments("p", Some(&json!("{\"a\": 1}"))).unwrap();
        assert_eq!(ok.get("a"), Some(&json!(1)));
        assert!(parse_arguments("p", Some(&json!(""))).unwrap().is_empty());
        assert!(parse_arguments("p", None).unwrap().is_empty());
        for bad in [json!("[1]"), json!("not json"), json!(7), json!([1])] {
            let error = parse_arguments("p", Some(&bad)).unwrap_err();
            assert!(!error.retryable, "{bad}");
        }
    }

    #[test]
    fn a_missing_or_unsafe_call_id_gets_a_unique_default_and_long_ids_are_cut() {
        let first = make_tool_call("p", 0, None, Some("t"), None).unwrap();
        let second = make_tool_call("p", 1, None, Some("t"), None).unwrap();
        assert_eq!(
            (first.id.as_str(), second.id.as_str()),
            ("call_0_t", "call_1_t")
        );
        for unsafe_id in ["a\u{1b}[31mb", "has space", "line\nbreak", "ñ"] {
            let call = make_tool_call("p", 2, Some(unsafe_id), Some("t"), None).unwrap();
            assert_eq!(call.id, "call_2_t", "{unsafe_id:?}");
        }
        let long = "i".repeat(500);
        let call = make_tool_call("p", 0, Some(&long), Some("t"), None).unwrap();
        assert_eq!(call.id.len(), 128);
    }

    #[test]
    fn provider_labels_are_cleaned_and_bounded() {
        assert_eq!(clean_field("gpt\u{1b}[31m-4\n\u{202e}"), "gpt[31m-4");
        assert_eq!(clean_field(&"m".repeat(1000)).len(), 200);
    }

    #[test]
    fn error_objects_map_status_and_retryability() {
        let data = json!({"error": {"message": "slow down", "code": 429}});
        let error = raise_if_error("p", data.as_object().unwrap(), None).unwrap_err();
        assert_eq!((error.status, error.retryable), (429, true));
        let data = json!({"error": {"message": "no balance", "code": 402}});
        let error = raise_if_error("p", data.as_object().unwrap(), None).unwrap_err();
        assert_eq!((error.status, error.retryable), (402, false));
        let data = json!({"error": "plain text"});
        let error = raise_if_error("p", data.as_object().unwrap(), None).unwrap_err();
        assert_eq!((error.status, error.retryable), (0, true));
        for harmless in [
            json!({}),
            json!({"error": null}),
            json!({"error": {}}),
            json!({"error": ""}),
        ] {
            assert!(raise_if_error("p", harmless.as_object().unwrap(), None).is_ok());
        }
    }

    #[test]
    fn usage_reads_the_named_counters() {
        let usage = parse_usage(Some(&json!({"a": 3, "b": 4})), "a", "b").unwrap();
        assert_eq!(
            (usage.prompt_tokens, usage.completion_tokens),
            (Some(3), Some(4))
        );
        assert_eq!(parse_usage(Some(&json!("x")), "a", "b"), None);
        assert_eq!(parse_usage(None, "a", "b"), None);
    }
}
