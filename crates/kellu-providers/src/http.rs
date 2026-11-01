//! HTTP transport for providers.
//!
//! Security properties (the same as the prototype's):
//!
//! * Endpoints must pass [`kellu_common::net::endpoint_problem`] (https, or http for localhost).
//!   The URL is parsed once by `http::Uri`, the parser the client itself uses.
//! * Redirects are refused, so an API key is never replayed to another host.
//! * Response bodies, individual SSE lines and total request duration are bounded.
//! * Error messages are scrubbed: control characters removed, the API key replaced, length
//!   limited.
//! * Keys travel only in headers built here from a [`Credential`], never in the URL.
//!
//! Proxies: https endpoints go through the proxy named by `ALL_PROXY`, `HTTPS_PROXY` or
//! `HTTP_PROXY` (and `NO_PROXY`) when one is set, as the prototype's `urllib` did; the proxy only
//! sees a `CONNECT` tunnel, so the key stays inside the TLS session. Plain-http endpoints (only
//! allowed for localhost) are **never** proxied: a proxy would see the key and the prompt in
//! cleartext. `ureq` has no implicit localhost exemption, so this is a separate client.

use std::io::{self, BufRead, BufReader, Read};
use std::num::NonZeroUsize;
use std::time::Duration;

use http::{HeaderName, HeaderValue, Method, Request};
use kellu_common::ProviderError;
use kellu_common::net::endpoint_problem;
use kellu_common::text::{strip_control_chars, truncate};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

/// Largest response body accepted.
pub const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
/// Longest SSE line accepted.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Longest a request (or a whole stream) may take.
pub const MAX_REQUEST_SECONDS: u64 = 600;
/// How much of an error body is read to explain a failure.
pub const ERROR_BODY_BYTES: u64 = 4096;

/// Longest scrubbed message.
const SCRUB_LIMIT: NonZeroUsize = NonZeroUsize::MIN.saturating_add(299);
/// Shortest key that is worth replacing in messages (avoids mangling text on tiny test keys).
const MIN_SCRUBBED_KEY_CHARS: usize = 8;

/// Size limits and durations; the defaults are the production values. Tests lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest response body (also the total size of a stream).
    pub max_response_bytes: u64,
    /// Longest single SSE line.
    pub max_line_bytes: usize,
    /// Longest a request or a stream may take in total.
    pub max_duration: Duration,
    /// How much of an error body is read.
    pub error_body_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_line_bytes: MAX_LINE_BYTES,
            max_duration: Duration::from_secs(MAX_REQUEST_SECONDS),
            error_body_bytes: ERROR_BODY_BYTES,
        }
    }
}

/// How the API key is presented to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>` (OpenAI-compatible APIs).
    Bearer,
    /// `x-goog-api-key: <key>` (Google).
    GoogleKey,
}

/// An API key and where it goes. `Debug` never shows the key.
pub struct Credential {
    style: AuthStyle,
    key: std::sync::Arc<SecretString>,
}

impl Credential {
    /// Present `key` the way `style` says.
    #[must_use]
    pub fn new(style: AuthStyle, key: std::sync::Arc<SecretString>) -> Self {
        Self { style, key }
    }

    fn secret(&self) -> &str {
        self.key.expose_secret()
    }

    fn header(&self) -> Result<(HeaderName, HeaderValue), ()> {
        let (name, value) = match self.style {
            AuthStyle::Bearer => (
                HeaderName::from_static("authorization"),
                format!("Bearer {}", self.secret()),
            ),
            AuthStyle::GoogleKey => (
                HeaderName::from_static("x-goog-api-key"),
                self.secret().to_owned(),
            ),
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| ())?;
        value.set_sensitive(true);
        Ok((name, value))
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("style", &self.style)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

/// Make `text` safe to show: no control characters, no key, a single line, bounded.
#[must_use]
pub fn scrub(text: &str, secret: Option<&str>) -> String {
    let mut cleaned = strip_control_chars(text);
    if let Some(secret) = secret
        && secret.chars().count() >= MIN_SCRUBBED_KEY_CHARS
    {
        cleaned = cleaned.replace(secret, "***");
    }
    let one_line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&one_line, SCRUB_LIMIT)
}

/// Whether another attempt or another provider may succeed after this HTTP status.
#[must_use]
pub fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 425 | 429) || status >= 500
}

fn message_from_body(body: &[u8], secret: Option<&str>) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(Value::Object(data)) = serde_json::from_str::<Value>(&text) {
        match data.get("error") {
            Some(Value::Object(error)) => {
                if let Some(Value::String(message)) = error.get("message") {
                    return scrub(message, secret);
                }
            }
            Some(Value::String(message)) => return scrub(message, secret),
            _ => {}
        }
    }
    scrub(&text, secret)
}

/// The ureq error, if `error` was produced by reading a ureq body.
fn ureq_cause(error: &io::Error) -> Option<&ureq::Error> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(ureq_cause(error), Some(ureq::Error::Timeout(_)))
        || matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        )
}

/// `ureq` reports TLS handshake failures (for example an untrusted or mismatched certificate) as
/// an `io::Error` of kind `InvalidData` that wraps the `rustls` error. Verified by hand against a
/// self-signed `openssl s_server`: the certificate is rejected and arrives in this shape.
fn is_tls_failure(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::InvalidData && error.get_ref().is_some()
}

fn is_too_large(error: &io::Error) -> bool {
    matches!(ureq_cause(error), Some(ureq::Error::BodyExceedsLimit(_)))
}

fn map_request_error(provider: &str, error: &ureq::Error, secret: Option<&str>) -> ProviderError {
    use ureq::Error as E;
    let detail = || scrub(&error.to_string(), secret);
    match error {
        E::Timeout(_) => ProviderError::new(provider, "request timed out"),
        E::BodyExceedsLimit(_) => {
            ProviderError::new(provider, "response too large").not_retryable()
        }
        E::Rustls(_) | E::Tls(_) | E::Pem(_) => {
            ProviderError::new(provider, format!("TLS error: {}", detail())).not_retryable()
        }
        E::BadUri(_) | E::RequireHttpsOnly(_) | E::TlsRequired | E::InvalidProxyUrl => {
            ProviderError::new(provider, "unsafe or invalid endpoint").not_retryable()
        }
        E::Http(_) | E::Protocol(_) | E::LargeResponseHeader(..) => {
            ProviderError::new(provider, format!("malformed HTTP response: {}", detail()))
                .not_retryable()
        }
        E::Io(io_error) if is_timeout(io_error) => {
            ProviderError::new(provider, "request timed out")
        }
        E::Io(io_error) if is_tls_failure(io_error) => {
            ProviderError::new(provider, format!("TLS error: {}", detail())).not_retryable()
        }
        _ => ProviderError::new(provider, format!("network error: {}", detail())),
    }
}

fn read_error(provider: &str, error: &io::Error, doing: &str) -> ProviderError {
    if is_too_large(error) {
        ProviderError::new(provider, "response too large").not_retryable()
    } else if is_timeout(error) {
        ProviderError::new(provider, format!("timed out while {doing}"))
    } else {
        ProviderError::new(provider, format!("connection lost while {doing}"))
    }
}

/// A blocking HTTP client with the transport rules above.
#[derive(Clone)]
pub struct Transport {
    /// For https endpoints; honors the proxy environment variables.
    agent: ureq::Agent,
    /// For plain-http (localhost) endpoints; never uses a proxy.
    direct: ureq::Agent,
    limits: Limits,
}

fn build_agent(timeout: Duration, limits: Limits, proxied: bool) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_resolve(Some(timeout))
        .timeout_connect(Some(timeout))
        .timeout_per_read(Some(timeout))
        .timeout_per_write(Some(timeout))
        .timeout_global(Some(limits.max_duration))
        .user_agent(concat!("kellu/", env!("CARGO_PKG_VERSION")));
    let builder = if proxied {
        builder
    } else {
        builder.proxy(None)
    };
    ureq::Agent::new_with_config(builder.build())
}

impl Transport {
    /// A client whose connect, read and write operations each time out after `timeout`.
    #[must_use]
    pub fn new(timeout: Duration, limits: Limits) -> Self {
        Self {
            agent: build_agent(timeout, limits, true),
            direct: build_agent(timeout, limits, false),
            limits,
        }
    }

    /// The limits this client enforces.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    fn send(
        &self,
        provider: &str,
        url: &str,
        headers: &[(&str, &str)],
        credential: Option<&Credential>,
        payload: &Value,
    ) -> Result<http::Response<ureq::Body>, ProviderError> {
        let secret = credential.map(Credential::secret);
        if let Some(problem) = endpoint_problem(url) {
            return Err(
                ProviderError::new(provider, format!("unsafe endpoint: {problem}")).not_retryable(),
            );
        }
        let invalid = || ProviderError::new(provider, "invalid request headers").not_retryable();
        let mut builder = Request::builder().method(Method::POST).uri(url);
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?;
            let value = HeaderValue::from_str(value).map_err(|_| invalid())?;
            builder = builder.header(name, value);
        }
        if let Some(credential) = credential {
            let (name, value) = credential.header().map_err(|()| {
                ProviderError::new(provider, "the stored API key is malformed").not_retryable()
            })?;
            builder = builder.header(name, value);
        }
        let request = builder
            .body(payload.to_string().into_bytes())
            .map_err(|_| invalid())?;

        let agent = if request.uri().scheme_str() == Some("https") {
            &self.agent
        } else {
            &self.direct
        };
        let response = agent
            .run(request)
            .map_err(|error| map_request_error(provider, &error, secret))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(
                ProviderError::new(provider, "redirect refused (has the endpoint moved?)")
                    .with_status(status)
                    .not_retryable(),
            );
        }
        if status >= 400 {
            let mut detail = Vec::new();
            // Best effort: a failure while reading the explanation does not hide the status.
            let _ = response
                .into_body()
                .into_reader()
                .take(self.limits.error_body_bytes)
                .read_to_end(&mut detail);
            let message = message_from_body(&detail, secret);
            let message = if message.is_empty() {
                format!("HTTP {status}")
            } else {
                message
            };
            return Err(ProviderError::new(provider, message)
                .with_status(status)
                .retryable(retryable_status(status)));
        }
        Ok(response)
    }

    /// POST `payload` as JSON and return the decoded JSON object.
    ///
    /// # Errors
    /// A scrubbed [`ProviderError`] for unsafe endpoints, network and TLS failures, HTTP errors,
    /// oversized bodies and bodies that are not a JSON object.
    pub fn post_json(
        &self,
        provider: &str,
        url: &str,
        headers: &[(&str, &str)],
        credential: Option<&Credential>,
        payload: &Value,
    ) -> Result<serde_json::Map<String, Value>, ProviderError> {
        let response = self.send(provider, url, headers, credential, payload)?;
        let raw = response
            .into_body()
            .with_config()
            .limit(self.limits.max_response_bytes)
            .read_to_vec()
            .map_err(|error| match error {
                ureq::Error::BodyExceedsLimit(_) => {
                    ProviderError::new(provider, "response too large").not_retryable()
                }
                ureq::Error::Timeout(_) => {
                    ProviderError::new(provider, "timed out while reading the response")
                }
                _ => ProviderError::new(provider, "connection lost while reading the response"),
            })?;
        match serde_json::from_slice::<Value>(&raw) {
            Ok(Value::Object(data)) => Ok(data),
            Ok(_) => Err(
                ProviderError::new(provider, "malformed response: expected an object")
                    .with_status(200)
                    .not_retryable(),
            ),
            Err(_) => Err(ProviderError::new(provider, "malformed response: not JSON")
                .with_status(200)
                .not_retryable()),
        }
    }

    /// POST `payload` and return the stream of server-sent events (the `data` of each event).
    ///
    /// The request is sent now; HTTP errors are returned here, read errors while iterating.
    ///
    /// # Errors
    /// The same request-level errors as [`Transport::post_json`].
    pub fn post_sse(
        &self,
        provider: &str,
        url: &str,
        headers: &[(&str, &str)],
        credential: Option<&Credential>,
        payload: &Value,
    ) -> Result<SseEvents, ProviderError> {
        let response = self.send(provider, url, headers, credential, payload)?;
        let reader = BufReader::new(response.into_body().into_reader());
        Ok(SseEvents::new(provider, Box::new(reader), self.limits))
    }
}

enum LineRead {
    Eof,
    Complete,
    TooLong,
}

/// Read one line (up to and including `\n`) of at most `limit` bytes into `buf`.
fn read_line_bounded(
    reader: &mut dyn BufRead,
    limit: usize,
    buf: &mut Vec<u8>,
) -> io::Result<LineRead> {
    buf.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                LineRead::Eof
            } else {
                LineRead::Complete
            });
        }
        let (take, found) = match available.iter().position(|&b| b == b'\n') {
            Some(index) => (index + 1, true),
            None => (available.len(), false),
        };
        if buf.len() + take > limit {
            return Ok(LineRead::TooLong);
        }
        buf.extend_from_slice(&available[..take]);
        reader.consume(take);
        if found {
            return Ok(LineRead::Complete);
        }
    }
}

/// Server-sent events: yields the `data` field of each event, joined with `\n` when it spans
/// several lines. Comments and keep-alives are skipped. After an error the iterator ends.
pub struct SseEvents {
    provider: String,
    reader: Box<dyn BufRead + Send>,
    limits: Limits,
    total: u64,
    data_lines: Vec<String>,
    line: Vec<u8>,
    finished: bool,
}

impl SseEvents {
    /// Parse events from `reader`, enforcing `limits`.
    #[must_use]
    pub fn new(provider: &str, reader: Box<dyn BufRead + Send>, limits: Limits) -> Self {
        Self {
            provider: provider.to_owned(),
            reader,
            limits,
            total: 0,
            data_lines: Vec::new(),
            line: Vec::new(),
            finished: false,
        }
    }

    fn fail(&mut self, error: ProviderError) -> Option<Result<String, ProviderError>> {
        self.finished = true;
        Some(Err(error))
    }

    fn flush(&mut self) -> Option<String> {
        if self.data_lines.is_empty() {
            return None;
        }
        let event = self.data_lines.join("\n");
        self.data_lines.clear();
        Some(event)
    }
}

impl Iterator for SseEvents {
    type Item = Result<String, ProviderError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            let mut buf = std::mem::take(&mut self.line);
            let read =
                read_line_bounded(self.reader.as_mut(), self.limits.max_line_bytes, &mut buf);
            let read = match read {
                Ok(read) => read,
                Err(error) => {
                    return self.fail(read_error(&self.provider, &error, "streaming"));
                }
            };
            match read {
                LineRead::Eof => {
                    self.finished = true;
                    return self.flush().map(Ok);
                }
                LineRead::TooLong => {
                    return self.fail(
                        ProviderError::new(&self.provider, "stream line too long").not_retryable(),
                    );
                }
                LineRead::Complete => {}
            }
            self.total += buf.len() as u64;
            if self.total > self.limits.max_response_bytes {
                return self.fail(
                    ProviderError::new(&self.provider, "response too large").not_retryable(),
                );
            }
            let text = String::from_utf8_lossy(&buf).into_owned();
            self.line = buf;
            let line = text.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if let Some(event) = self.flush() {
                    return Some(Ok(event));
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (name, value) = line.split_once(':').unwrap_or((line, ""));
            if name == "data" {
                self.data_lines
                    .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(input: &str, limits: Limits) -> Vec<Result<String, String>> {
        let reader = Box::new(io::Cursor::new(input.as_bytes().to_vec()));
        SseEvents::new("p", reader, limits)
            .map(|item| item.map_err(|e| e.reason))
            .collect()
    }

    #[test]
    fn scrub_removes_controls_the_key_and_extra_space() {
        let text = "bad \x1b[31mkey sk-secret-1234 \n\n here\u{202e}";
        let clean = scrub(text, Some("sk-secret-1234"));
        assert_eq!(clean, "bad [31mkey *** here");
        assert!(!clean.contains("sk-secret"));
    }

    #[test]
    fn scrub_leaves_very_short_keys_alone_and_bounds_length() {
        assert_eq!(scrub("abc abc", Some("abc")), "abc abc");
        let long = "x".repeat(1000);
        assert!(scrub(&long, None).chars().count() < 340);
    }

    #[test]
    fn retryable_statuses_match_the_prototype() {
        for status in [408, 409, 425, 429, 500, 502, 503, 599] {
            assert!(retryable_status(status), "{status}");
        }
        for status in [400, 401, 402, 403, 404, 422] {
            assert!(!retryable_status(status), "{status}");
        }
    }

    #[test]
    fn sse_joins_data_lines_and_skips_comments() {
        let input = ": keep-alive\ndata: one\n\ndata: two\ndata:three\n\nevent: x\n\n";
        assert_eq!(
            events(input, Limits::default()),
            vec![Ok("one".to_owned()), Ok("two\nthree".to_owned())]
        );
    }

    #[test]
    fn sse_flushes_a_final_event_without_a_blank_line_and_handles_crlf() {
        let input = "data: a\r\n\r\ndata: b";
        assert_eq!(
            events(input, Limits::default()),
            vec![Ok("a".to_owned()), Ok("b".to_owned())]
        );
    }

    #[test]
    fn a_line_over_the_limit_is_an_error_and_ends_the_stream() {
        let limits = Limits {
            max_line_bytes: 16,
            ..Limits::default()
        };
        let input = format!("data: ok\n\ndata: {}\n\ndata: later\n\n", "x".repeat(64));
        let got = events(&input, limits);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Ok("ok".to_owned()));
        assert_eq!(got[1], Err("stream line too long".to_owned()));
    }

    #[test]
    fn a_stream_over_the_total_limit_is_an_error() {
        let limits = Limits {
            max_response_bytes: 32,
            ..Limits::default()
        };
        let input = "data: 0123456789\n\n".repeat(5);
        let got = events(&input, limits);
        assert_eq!(got.last(), Some(&Err("response too large".to_owned())));
    }

    #[test]
    fn request_errors_are_classified_for_the_router() {
        let classify = |error: ureq::Error| map_request_error("p", &error, Some("sk-secret-1234"));
        let cert = io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid peer certificate: sk-secret-1234",
        );
        let tls = classify(ureq::Error::Io(cert));
        assert!(tls.reason.starts_with("TLS error"), "{}", tls.reason);
        assert!(!tls.retryable);
        assert!(!tls.reason.contains("sk-secret"));
        let eof = classify(ureq::Error::Io(io::Error::from(
            io::ErrorKind::UnexpectedEof,
        )));
        assert!(eof.reason.starts_with("network error") && eof.retryable);
        let refused = classify(ureq::Error::Io(io::Error::from(
            io::ErrorKind::ConnectionRefused,
        )));
        assert!(refused.retryable);
        let timed_out = classify(ureq::Error::Io(io::Error::from(io::ErrorKind::TimedOut)));
        assert!(timed_out.reason == "request timed out" && timed_out.retryable);
        assert!(!classify(ureq::Error::BodyExceedsLimit(1)).retryable);
        assert!(!classify(ureq::Error::TlsRequired).retryable);
        assert!(classify(ureq::Error::HostNotFound).retryable);
        assert!(classify(ureq::Error::ConnectionFailed).retryable);
    }

    #[test]
    fn credentials_never_show_the_key_in_debug_output() {
        let key = std::sync::Arc::new(SecretString::from("sk-very-secret-key".to_owned()));
        let credential = Credential::new(AuthStyle::Bearer, key);
        assert!(!format!("{credential:?}").contains("sk-very"));
    }

    #[test]
    fn a_key_that_cannot_be_a_header_is_refused() {
        let key = std::sync::Arc::new(SecretString::from("bad\r\nX-Evil: 1".to_owned()));
        assert!(Credential::new(AuthStyle::Bearer, key).header().is_err());
    }
}
