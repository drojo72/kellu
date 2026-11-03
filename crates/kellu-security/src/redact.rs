//! Redactor v1: removes secrets from text before it is logged, stored or sent.
//!
//! Best effort, and documented as such in `SECURITY.md`: it knows the formats of common tokens,
//! private key blocks, `Authorization` and `key=value` assignments, exact values of keys the
//! caller registered, and long high-entropy tokens. It misses things. It is a second line of
//! defence behind "secrets are never put in the context in the first place".
//!
//! The scanner is hand-written and linear in the size of the text; it never panics on any
//! input (every slice is cut on `char` boundaries).

use kellu_common::text::Redactor;
use secrecy::{ExposeSecret, SecretString};

/// What a redacted secret is replaced with.
pub const PLACEHOLDER: &str = "[REDACTED]";

/// Known secrets shorter than this are not matched by value (too many false positives).
const MIN_KNOWN_SECRET_LEN: usize = 8;
const MIN_ENTROPY_LEN: usize = 24;
const ENTROPY_THRESHOLD: f64 = 4.0;

/// `(prefix, minimum characters after the prefix)` of well-known token formats.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("sk-", 16),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("glpat-", 16),
    ("xai-", 16),
    ("AIza", 30),
    ("AKIA", 12),
    ("ASIA", 12),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("xoxa-", 10),
    ("xoxr-", 10),
    ("xoxs-", 10),
    ("napi_", 16),
    ("rnd_", 16),
    ("cfut_", 16),
];

/// Names whose assigned value is redacted (`name = value`, `name: value`, JSON `"name": "value"`).
const SECRET_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "token",
    "secret",
    "password",
    "passwd",
    "authorization",
    "bearer",
    "access_key",
    "private_key",
    "client_secret",
];

/// Removes secrets by pattern, entropy and (optionally) by exact value.
#[derive(Clone, Default)]
pub struct PatternRedactor {
    known: Vec<SecretString>,
}

impl std::fmt::Debug for PatternRedactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternRedactor")
            .field("known_secrets", &self.known.len())
            .finish()
    }
}

impl PatternRedactor {
    /// A redactor that uses patterns and entropy only.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Also remove every occurrence of this exact value (for example a key just loaded from the
    /// vault). Values shorter than 8 characters are ignored.
    #[must_use]
    pub fn with_known_secret(mut self, secret: SecretString) -> Self {
        if secret.expose_secret().chars().count() >= MIN_KNOWN_SECRET_LEN {
            self.known.push(secret);
        }
        self
    }
}

impl Redactor for PatternRedactor {
    fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for secret in &self.known {
            out = out.replace(secret.expose_secret(), PLACEHOLDER);
        }
        out = redact_private_keys(&out);
        out = redact_assignments(&out);
        redact_tokens(&out)
    }
}

// ----------------------------------------------------------------------- private keys ----

fn redact_private_keys(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const KEY_MARK: &str = "PRIVATE KEY-----";
    const END: &str = "-----END ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let header_ok = after
            .find("-----")
            .is_some_and(|i| after[..i + 5].ends_with(KEY_MARK));
        if !header_ok {
            out.push_str(&rest[..start + BEGIN.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(PLACEHOLDER);
        // Drop through the matching END line; an unterminated block loses the rest of the text.
        rest = match after.find(END) {
            Some(end) => {
                let tail = &after[end + END.len()..];
                tail.find("-----").map_or("", |i| &tail[i + 5..])
            }
            None => "",
        };
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------------ assignments ----

fn is_value_char(c: char) -> bool {
    !(c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '}' | ']' | ')' | '<' | '>'))
}

/// Authentication schemes that precede the real credential (`Authorization: Bearer <token>`).
const SCHEMES: &[&str] = &["bearer ", "token ", "basic "];

/// Redact the value after `name = `, `name: ` or `"name": "` for the names in [`SECRET_NAMES`],
/// and after `Bearer `. Values that are already placeholders are left alone.
fn redact_assignments(text: &str) -> String {
    let lower = text.to_ascii_lowercase(); // same byte offsets as `text`
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (at, name_len) in name_hits(&lower) {
        if at < cursor {
            continue; // inside a value that was already redacted
        }
        let Some(mut start) = value_start(text, &lower, at + name_len) else {
            continue;
        };
        if let Some(scheme) = SCHEMES.iter().find(|s| lower[start..].starts_with(**s)) {
            start += scheme.len();
        }
        let len: usize = text[start..]
            .chars()
            .take_while(|c| is_value_char(*c))
            .map(char::len_utf8)
            .sum();
        let value = &text[start..start + len];
        if len < 6 || value.starts_with(PLACEHOLDER) || value.starts_with('[') {
            continue;
        }
        out.push_str(&text[cursor..start]);
        out.push_str(PLACEHOLDER);
        cursor = start + len;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Every occurrence of a secret name, as `(offset, length)`, in order of position. One pass per
/// name keeps the whole scan linear in the size of the text.
fn name_hits(lower: &str) -> Vec<(usize, usize)> {
    let mut hits = Vec::new();
    for name in SECRET_NAMES.iter().chain(&["bearer"]) {
        let mut from = 0;
        while let Some(i) = lower.get(from..).and_then(|rest| rest.find(name)) {
            let at = from + i;
            let end = at + name.len();
            // A name must end at a word boundary, so `tokenizer` is not `token`.
            let boundary = lower[end..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric());
            if boundary || *name == "bearer" {
                hits.push((at, name.len()));
            }
            from = end;
        }
    }
    hits.sort_unstable();
    hits
}

/// Where the value starts after a secret name: skips closing quote, `=`, `:` and spaces. `None`
/// when no assignment follows (the name was just mentioned). `Bearer` needs only a space.
fn value_start(text: &str, lower: &str, after_name: usize) -> Option<usize> {
    let mut seen_separator = false;
    let mut end = after_name;
    for c in text[after_name..].chars() {
        match c {
            '"' | '\'' | ' ' | '\t' => {}
            '=' | ':' => seen_separator = true,
            _ => break,
        }
        end += c.len_utf8();
    }
    let bearer_gap = lower[..after_name].ends_with("bearer") && end > after_name;
    (seen_separator || bearer_gap).then_some(end)
}

// -------------------------------------------------------------------------- tokens ----

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.')
}

fn redact_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token_start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_token_char(c) {
            token_start.get_or_insert(i);
        } else if let Some(start) = token_start.take() {
            push_token(&mut out, &text[start..i]);
            out.push(c);
        } else {
            out.push(c);
        }
    }
    if let Some(start) = token_start {
        push_token(&mut out, &text[start..]);
    }
    out
}

fn push_token(out: &mut String, token: &str) {
    if token.starts_with(PLACEHOLDER) || !is_secret_token(token) {
        out.push_str(token);
    } else {
        out.push_str(PLACEHOLDER);
    }
}

fn is_secret_token(token: &str) -> bool {
    if token.starts_with("eyJ") && is_jwt(token) {
        return true;
    }
    if TOKEN_PREFIXES.iter().any(|(prefix, min)| {
        token
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.len() >= *min)
    }) {
        return true;
    }
    looks_high_entropy(token)
}

/// `header.payload.signature`, each part base64url, the first two starting with `eyJ`.
fn is_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts[0].starts_with("eyJ")
        && parts[1].starts_with("eyJ")
        && parts.iter().all(|p| {
            p.len() >= 8
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

fn looks_high_entropy(token: &str) -> bool {
    if token.len() < MIN_ENTROPY_LEN || !token.is_ascii() {
        return false;
    }
    // Paths and URLs contain `/` and `.` in structure; a secret is one dense run.
    if token.matches('/').count() > 2 || token.matches('.').count() > 2 {
        return false;
    }
    let has_digit = token.bytes().any(|b| b.is_ascii_digit());
    let has_letter = token.bytes().any(|b| b.is_ascii_alphabetic());
    if !(has_digit && has_letter) {
        return false;
    }
    shannon_entropy(token.as_bytes()) >= ENTROPY_THRESHOLD
}

fn shannon_entropy(bytes: &[u8]) -> f64 {
    let mut counts = [0_u32; 256];
    for byte in bytes {
        counts[usize::from(*byte)] += 1;
    }
    let total = f64::from(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let p = f64::from(*count) / total;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(text: &str) -> String {
        PatternRedactor::new().redact(text)
    }

    fn assert_hidden(text: &str, secret: &str) {
        let out = redact(text);
        assert!(!out.contains(secret), "leaked {secret:?} in {out:?}");
        assert!(out.contains(PLACEHOLDER), "no placeholder in {out:?}");
    }

    #[test]
    fn well_known_token_formats_are_removed() {
        for token in [
            "sk-proj-abcdefghijklmnopqrstuvwx",
            "sk-or-v1-0123456789abcdef0123456789abcdef",
            "ghp_0123456789abcdefghijABCDEFGHIJ",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "xai-0123456789abcdefghijklmnop",
            "AIzaSyA-0123456789abcdefghijklmnopqrstu",
            "AKIAIOSFODNN7EXAMPLE",
            "xoxb-123456789012-abcdef",
            "glpat-abcdefghij0123456789",
        ] {
            assert_hidden(&format!("the key is {token} ok"), token);
        }
    }

    #[test]
    fn a_jwt_is_removed() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_hidden(&format!("token {jwt}"), jwt);
    }

    #[test]
    fn private_key_blocks_are_removed_whole() {
        let pem = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0123\nabcd\n-----END RSA PRIVATE KEY-----\nafter";
        let out = redact(pem);
        assert!(!out.contains("MIIEow") && !out.contains("abcd"), "{out}");
        assert!(
            out.starts_with("before\n") && out.ends_with("\nafter"),
            "{out}"
        );
    }

    #[test]
    fn an_unterminated_private_key_block_loses_the_rest() {
        let out = redact("x\n-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkq\nmore stuff");
        assert!(
            !out.contains("MIIEvQ") && !out.contains("more stuff"),
            "{out}"
        );
    }

    #[test]
    fn public_key_blocks_are_not_secrets() {
        let text = "-----BEGIN PUBLIC KEY-----\nMFkw\n-----END PUBLIC KEY-----";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn assignments_and_headers_are_removed() {
        assert_hidden("api_key = hunter2hunter2", "hunter2hunter2");
        assert_hidden("API_KEY=hunter2hunter2", "hunter2hunter2");
        assert_hidden("password: correct-horse-battery", "correct-horse");
        assert_hidden(r#"{"token": "abcdef123456", "x": 1}"#, "abcdef123456");
        assert_hidden("Authorization: Bearer abcdef0123456789", "abcdef0123456789");
        assert_hidden(
            "curl -H 'authorization: token notasecretatall1'",
            "notasecretatall1",
        );
        assert_hidden("Bearer abcdef0123", "abcdef0123");
    }

    #[test]
    fn the_name_and_the_rest_of_the_line_stay_readable() {
        let out = redact("api_key = hunter2hunter2, retries = 3");
        assert_eq!(out, "api_key = [REDACTED], retries = 3");
    }

    #[test]
    fn mentioning_a_name_is_not_an_assignment() {
        for text in [
            "the token budget is large",
            "rotate your api_key regularly",
            "tokenizer settings",
            "secret handling is documented",
            "password policy",
        ] {
            assert_eq!(redact(text), text);
        }
    }

    #[test]
    fn short_values_and_placeholders_are_left_alone() {
        assert_eq!(redact("token = 1234"), "token = 1234");
        assert_eq!(redact("api_key = [REDACTED]"), "api_key = [REDACTED]");
    }

    #[test]
    fn exact_values_of_known_secrets_are_removed_everywhere() {
        let redactor = PatternRedactor::new()
            .with_known_secret(SecretString::from("plain-value-42".to_owned()));
        let out = redactor.redact("error: plain-value-42 rejected; retry with plain-value-42");
        assert_eq!(out, "error: [REDACTED] rejected; retry with [REDACTED]");
    }

    #[test]
    fn short_known_secrets_are_not_matched_by_value() {
        let redactor =
            PatternRedactor::new().with_known_secret(SecretString::from("abc".to_owned()));
        assert_eq!(redactor.redact("abc abc"), "abc abc");
    }

    #[test]
    fn dense_random_tokens_are_removed_by_entropy() {
        assert_hidden(
            "value Zk3vQ9xLw2PmN7bT4cRy8HaJ5dFg6EsU end",
            "Zk3vQ9xLw2PmN7bT4cRy8HaJ5dFg6EsU",
        );
    }

    #[test]
    fn ordinary_text_is_not_mistaken_for_a_secret() {
        for text in [
            "The quick brown fox jumps over the lazy dog",
            "commit 3f2a9c1d8e7b6a5f4e3d2c1b0a9f8e7d6c5b4a39 fixed it",
            "/usr/local/lib/python3.11/site-packages/package/module.py",
            "https://example.com/a/b/c?x=1",
            "UPPERCASEONLYIDENTIFIERWITHNODIGITS",
            "fn very_long_function_name_that_does_things(arg: u32) -> u32",
            "2026-11-02T09:47:00Z",
            "kellu-security",
        ] {
            assert_eq!(redact(text), text, "{text}");
        }
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact("api_key = hunter2hunter2 and sk-abcdefghijklmnopqrstuvwxyz");
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn unicode_and_odd_input_never_panic() {
        for text in [
            "",
            "ñandú token = ñandúñandú",
            "🔑 api_key=🔑🔑🔑🔑🔑🔑🔑",
            "token=",
            "token = \"",
            "bearer",
            "-----BEGIN ",
            "-----BEGIN PRIVATE KEY-----",
            "\0\0\0",
            "é".repeat(500).as_str(),
        ] {
            let _ = redact(text);
        }
    }

    #[test]
    fn it_works_as_a_redactor_behind_the_hooks() {
        let hook: &dyn Redactor = &PatternRedactor::new();
        assert_eq!(hook.redact("nothing here"), "nothing here");
    }

    #[test]
    fn debug_never_shows_known_secrets() {
        let redactor = PatternRedactor::new()
            .with_known_secret(SecretString::from("plain-value-42".to_owned()));
        assert!(!format!("{redactor:?}").contains("plain-value"));
    }
}
