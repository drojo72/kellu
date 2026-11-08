//! Reading the JSON arguments of a tool call, strictly.
//!
//! Unknown keys are an error (the equivalent of `deny_unknown_fields`). Errors name the key and
//! never repeat the value.

use serde_json::{Map, Value};

use crate::spec::ToolError;

/// The longest string accepted for any argument.
pub const MAX_STRING_BYTES: usize = 1 << 20;

/// The arguments of one call.
#[derive(Debug, Clone, Copy)]
pub struct Args<'a> {
    map: &'a Map<String, Value>,
}

impl<'a> Args<'a> {
    /// `value` must be an object whose keys are all in `allowed`.
    ///
    /// # Errors
    /// It is not an object, or has a key that is not allowed.
    pub fn new(value: &'a Value, allowed: &[&str]) -> Result<Self, ToolError> {
        let map = value
            .as_object()
            .ok_or_else(|| ToolError::Invalid("the arguments must be a JSON object".into()))?;
        let unknown: Vec<String> = map
            .keys()
            .filter(|key| !allowed.contains(&key.as_str()))
            .map(|key| {
                key.chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .take(40)
                    .collect()
            })
            .collect();
        if unknown.is_empty() {
            Ok(Self { map })
        } else {
            Err(ToolError::Invalid(format!(
                "unknown argument(s): {}",
                unknown.join(", ")
            )))
        }
    }

    /// Whether `name` is present and not `null`.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.map.get(name).is_some_and(|v| !v.is_null())
    }

    /// A required string.
    ///
    /// # Errors
    /// Missing, not a string, too long or containing a NUL byte.
    pub fn str(&self, name: &str) -> Result<&'a str, ToolError> {
        self.opt_str(name)?
            .ok_or_else(|| ToolError::Invalid(format!("{name}: required")))
    }

    /// An optional string (absent or `null` is `None`).
    ///
    /// # Errors
    /// Not a string, too long or containing a NUL byte.
    pub fn opt_str(&self, name: &str) -> Result<Option<&'a str>, ToolError> {
        match self.map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => {
                if text.len() > MAX_STRING_BYTES {
                    Err(ToolError::Invalid(format!("{name}: too long")))
                } else if text.contains('\0') {
                    Err(ToolError::Invalid(format!("{name}: contains a NUL byte")))
                } else {
                    Ok(Some(text))
                }
            }
            Some(_) => Err(ToolError::Invalid(format!("{name}: expected a string"))),
        }
    }

    /// An optional boolean with a default.
    ///
    /// # Errors
    /// Present but not a boolean.
    pub fn bool_or(&self, name: &str, default: bool) -> Result<bool, ToolError> {
        match self.map.get(name) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(_) => Err(ToolError::Invalid(format!("{name}: expected a boolean"))),
        }
    }

    /// An optional non-negative integer.
    ///
    /// # Errors
    /// Present but not a non-negative integer.
    pub fn opt_u64(&self, name: &str) -> Result<Option<u64>, ToolError> {
        match self.map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value.as_u64().map(Some).ok_or_else(|| {
                ToolError::Invalid(format!("{name}: expected a non-negative integer"))
            }),
        }
    }

    /// A required array with at most `max` items.
    ///
    /// # Errors
    /// Missing, not an array, or too long.
    pub fn array(&self, name: &str, max: usize) -> Result<&'a [Value], ToolError> {
        match self.map.get(name) {
            Some(Value::Array(items)) if items.len() <= max => Ok(items),
            Some(Value::Array(_)) => {
                Err(ToolError::Invalid(format!("{name}: at most {max} items")))
            }
            Some(_) => Err(ToolError::Invalid(format!("{name}: expected an array"))),
            None => Err(ToolError::Invalid(format!("{name}: required"))),
        }
    }

    /// A required array of non-empty strings.
    ///
    /// # Errors
    /// Missing, not an array of strings, empty, too long, or a string has a NUL byte.
    pub fn str_list(&self, name: &str, max: usize) -> Result<Vec<&'a str>, ToolError> {
        let items = self.array(name, max)?;
        if items.is_empty() {
            return Err(ToolError::Invalid(format!("{name}: must not be empty")));
        }
        items
            .iter()
            .map(|item| match item {
                Value::String(text) if !text.contains('\0') && text.len() <= 4096 => {
                    Ok(text.as_str())
                }
                _ => Err(ToolError::Invalid(format!(
                    "{name}: expected short strings without NUL bytes"
                ))),
            })
            .collect()
    }
}

/// An object inside an array (for example one edit), with the same strictness.
///
/// # Errors
/// As [`Args::new`].
pub fn object<'a>(value: &'a Value, allowed: &[&str]) -> Result<Args<'a>, ToolError> {
    Args::new(value, allowed)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn unknown_keys_are_refused_and_named_without_their_values() {
        let value = json!({"path": "a", "secret_token_value": "hunter2hunter2"});
        let error = Args::new(&value, &["path"]).unwrap_err().to_string();
        assert!(error.contains("secret_token_value"));
        assert!(!error.contains("hunter2"));
    }

    #[test]
    fn types_and_limits_are_enforced() {
        let value = json!({"s": 1, "b": "x", "n": -1, "a": [1], "nul": "a\u{0}b"});
        let args = Args::new(&value, &["s", "b", "n", "a", "nul", "missing"]).unwrap();
        assert!(args.str("s").is_err());
        assert!(args.bool_or("b", false).is_err());
        assert!(args.opt_u64("n").is_err());
        assert!(args.str_list("a", 5).is_err());
        assert!(args.str("nul").is_err());
        assert!(args.str("missing").is_err());
        assert!(args.opt_str("missing").unwrap().is_none());
        assert!(args.array("a", 0).is_err());
    }

    #[test]
    fn a_non_object_is_refused() {
        assert!(Args::new(&json!([1]), &[]).is_err());
        assert!(Args::new(&json!("x"), &[]).is_err());
    }
}
