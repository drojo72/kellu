//! Task classification and provider routing with fallback.
//!
//! The router walks the configured chain for a task type and moves to the next provider when one
//! fails. Rules worth knowing:
//!
//! * A provider that cannot do what the request needs (tools) is skipped, not tried.
//! * No new attempt starts after the total time budget is spent.
//! * Any provider error moves on to the next provider, including non-retryable ones such as
//!   HTTP 402 (no balance): a different provider may well succeed.
//! * A stream can fail over only **before** any text was delivered. After that, switching
//!   providers would duplicate or contradict what the user already saw, so the error is raised.
//! * Failure messages come from the providers and never contain secrets.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use kellu_common::{KelluError, ProviderError, ProviderFailure};
use kellu_providers::{
    ChatRequest, ChatResponse, EventStream, KeyLookup, Provider, StreamEvent, build_provider,
};

use crate::config::Config;

/// Seconds on a monotonic clock; injectable so tests can move time by hand.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// A monotonic clock that starts at zero when it is created.
#[must_use]
pub fn monotonic_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed().as_secs_f64())
}

/// The kind of work a prompt asks for; each kind has its own provider chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// Anything else.
    General,
    /// Programming questions and code.
    Code,
    /// Explanations of concepts.
    Explain,
}

impl TaskKind {
    /// The route name used in the configuration (`[routing.<name>]`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Code => "code",
            Self::Explain => "explain",
        }
    }
}

const EXPLAIN_PREFIXES: &[&str] = &[
    "explain",
    "explica",
    "what is",
    "what are",
    "how does",
    "how do",
    "why ",
    "que es",
    "que son",
    "como funciona",
    "por que",
];

const CODE_WORDS: &[&str] = &[
    "bug",
    "traceback",
    "exception",
    "stacktrace",
    "refactor",
    "refactoriza",
    "compile",
    "compilar",
    "function",
    "funcion",
    "script",
    "regex",
    "sql",
    "python",
    "javascript",
    "rust",
    "bash",
    "git",
    "codigo",
    "code",
    "clase",
    "class",
    "typescript",
    "docker",
];

const CODE_LINE_STARTS: &[&str] = &[
    "def", "class", "import", "from", "const", "let", "fn", "func", "#include",
];

/// Lowercase and strip accents so Spanish and English match the same word lists.
///
/// A small explicit table covers the Latin letters Spanish and its neighbours use; combining
/// marks (decomposed input such as `e` + U+0301) are dropped. This replaces the prototype's
/// Unicode normalization without a new dependency.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn starts_with_keyword(line: &str, keyword: &str) -> bool {
    line.strip_prefix(keyword)
        .is_some_and(|rest| !rest.chars().next().is_some_and(is_word_char))
}

fn has_code_line(folded: &str) -> bool {
    folded.lines().any(|line| {
        let line = line.trim_start();
        CODE_LINE_STARTS
            .iter()
            .any(|keyword| starts_with_keyword(line, keyword))
    })
}

/// Classify a user prompt (Spanish or English).
#[must_use]
pub fn classify_task(prompt: &str) -> TaskKind {
    let folded = fold(prompt);
    if folded.contains("```") || has_code_line(&folded) {
        return TaskKind::Code;
    }
    let start = folded.trim_start();
    if EXPLAIN_PREFIXES
        .iter()
        .any(|prefix| start.starts_with(prefix))
    {
        return TaskKind::Explain;
    }
    let is_word = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '#' | '+');
    if folded
        .split(|c: char| !is_word(c))
        .any(|word| CODE_WORDS.contains(&word))
    {
        return TaskKind::Code;
    }
    TaskKind::General
}

fn failure(name: &str, status: u16, message: &str, retryable: bool) -> ProviderFailure {
    ProviderFailure {
        provider: name.to_owned(),
        status,
        message: message.to_owned(),
        retryable,
    }
}

fn record(name: &str, error: &ProviderError) -> ProviderFailure {
    log::warn!(
        "provider {name} failed (status {}, retryable={}): {}",
        error.status,
        error.retryable,
        error.reason
    );
    failure(name, error.status, &error.reason, error.retryable)
}

/// The providers worth trying for one request, in chain order. Skipped providers are recorded
/// as failures; the time budget is checked before each new attempt.
struct Attempts<'a> {
    providers: &'a BTreeMap<String, Box<dyn Provider>>,
    chain: Box<dyn Iterator<Item = &'a str> + 'a>,
    clock: Clock,
    deadline: f64,
    needs_tools: bool,
}

impl<'a> Attempts<'a> {
    fn next(&mut self, failures: &mut Vec<ProviderFailure>) -> Option<(&'a str, &'a dyn Provider)> {
        for name in self.chain.by_ref() {
            let Some(provider) = self.providers.get(name) else {
                failures.push(failure(name, 0, "provider is not available", false));
                continue;
            };
            if self.needs_tools && !provider.supports_tools() {
                failures.push(failure(name, 0, "does not support tool calling", false));
                continue;
            }
            if (self.clock)() >= self.deadline {
                failures.push(failure(name, 0, "total time budget exhausted", false));
                return None;
            }
            return Some((name, provider.as_ref()));
        }
        None
    }
}

/// Routes chat requests to providers according to the configuration.
pub struct Router {
    config: Config,
    providers: BTreeMap<String, Box<dyn Provider>>,
    clock: Clock,
}

impl Router {
    /// A router over explicit providers (tests inject fakes).
    #[must_use]
    pub fn new(
        config: Config,
        providers: BTreeMap<String, Box<dyn Provider>>,
        clock: Clock,
    ) -> Self {
        Self {
            config,
            providers,
            clock,
        }
    }

    /// Build every configured provider. Keys are looked up per request, not here.
    #[must_use]
    pub fn from_config(config: Config, key_lookup: &KeyLookup, clock: Clock) -> Self {
        let providers = config
            .providers()
            .iter()
            .map(|(name, provider)| (name.clone(), build_provider(provider, key_lookup)))
            .collect();
        Self::new(config, providers, clock)
    }

    /// Names of the providers that were built, sorted.
    pub fn provider_names(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    fn attempts(&self, task: TaskKind, request: &ChatRequest) -> Attempts<'_> {
        Attempts {
            providers: &self.providers,
            chain: Box::new(self.config.route(task.as_str()).chain()),
            clock: Arc::clone(&self.clock),
            deadline: (self.clock)() + self.config.router().total_timeout_s,
            needs_tools: !request.tools.is_empty(),
        }
    }

    /// Return the first successful answer along the chain for `task`.
    ///
    /// # Errors
    /// [`KelluError::AllProvidersFailed`] listing every provider tried or skipped.
    pub fn chat(&self, task: TaskKind, request: &ChatRequest) -> Result<ChatResponse, KelluError> {
        let mut failures = Vec::new();
        let mut attempts = self.attempts(task, request);
        while let Some((name, provider)) = attempts.next(&mut failures) {
            match provider.chat(request) {
                Ok(response) => return Ok(response),
                Err(error) => failures.push(record(name, &error)),
            }
        }
        Err(KelluError::AllProvidersFailed {
            task: task.as_str().to_owned(),
            failures,
        })
    }

    /// Stream the answer. Fails over only until the first text has been delivered.
    #[must_use]
    pub fn stream(&self, task: TaskKind, request: &ChatRequest) -> RouterStream<'_> {
        RouterStream {
            attempts: self.attempts(task, request),
            task,
            request: request.clone(),
            failures: Vec::new(),
            current: None,
            ended: false,
        }
    }
}

struct Current {
    name: String,
    stream: EventStream,
    delivered: bool,
    finished: bool,
}

/// The events of a routed stream: text deltas, then one `Done`; or an error.
pub struct RouterStream<'a> {
    attempts: Attempts<'a>,
    task: TaskKind,
    request: ChatRequest,
    failures: Vec<ProviderFailure>,
    current: Option<Current>,
    ended: bool,
}

impl RouterStream<'_> {
    /// A provider failed: raise it if the user already saw text, otherwise note it and move on.
    fn failed(&mut self, name: &str, error: ProviderError, delivered: bool) -> Option<KelluError> {
        self.current = None;
        let failure = record(name, &error);
        if delivered {
            self.ended = true;
            return Some(KelluError::Provider(error));
        }
        self.failures.push(failure);
        None
    }
}

impl Iterator for RouterStream<'_> {
    type Item = Result<StreamEvent, KelluError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.ended {
                return None;
            }
            let Some(current) = self.current.as_mut() else {
                match self.attempts.next(&mut self.failures) {
                    Some((name, provider)) => {
                        self.current = Some(Current {
                            name: name.to_owned(),
                            stream: provider.stream(&self.request),
                            delivered: false,
                            finished: false,
                        });
                        continue;
                    }
                    None => {
                        self.ended = true;
                        return Some(Err(KelluError::AllProvidersFailed {
                            task: self.task.as_str().to_owned(),
                            failures: std::mem::take(&mut self.failures),
                        }));
                    }
                }
            };
            match current.stream.next() {
                Some(Ok(event)) => {
                    match &event {
                        StreamEvent::Text(text) if !text.is_empty() => current.delivered = true,
                        StreamEvent::Done(_) => current.finished = true,
                        StreamEvent::Text(_) => {}
                    }
                    return Some(Ok(event));
                }
                None if current.finished => {
                    self.ended = true;
                    return None;
                }
                outcome => {
                    let error = match outcome {
                        Some(Err(error)) => error,
                        _ => {
                            ProviderError::new(&current.name, "stream ended without a final event")
                        }
                    };
                    let (name, delivered) = (current.name.clone(), current.delivered);
                    if let Some(error) = self.failed(&name, error, delivered) {
                        return Some(Err(error));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_task_handles_spanish_and_english() {
        let cases = [
            ("Explain how TCP works", TaskKind::Explain),
            ("explícame qué es un monad", TaskKind::Explain),
            ("¿Qué es un puntero?", TaskKind::General),
            ("qué es un puntero", TaskKind::Explain),
            ("Qué es un puntero", TaskKind::Explain),
            ("Cómo funciona el borrow checker", TaskKind::Explain),
            ("¿Por qué falla?", TaskKind::General),
            ("Por qué falla", TaskKind::Explain),
            ("fix this bug in my script", TaskKind::Code),
            ("arregla este error de compilación en rust", TaskKind::Code),
            ("tengo un problema con docker", TaskKind::Code),
            ("```\nlet x = 1;\n```", TaskKind::Code),
            ("fn main() {}", TaskKind::Code),
            ("  import os", TaskKind::Code),
            ("#include <stdio.h>", TaskKind::Code),
            ("hola, ¿cómo estás?", TaskKind::General),
            ("recomiéndame una película", TaskKind::General),
            ("", TaskKind::General),
        ];
        for (prompt, expected) in cases {
            assert_eq!(classify_task(prompt), expected, "{prompt:?}");
        }
    }

    #[test]
    fn keyword_lines_need_a_word_boundary() {
        for prompt in [
            "important note",
            "classic rock",
            "letter to a friend",
            "fnord",
        ] {
            assert_eq!(classify_task(prompt), TaskKind::General, "{prompt:?}");
        }
    }

    #[test]
    fn accents_fold_in_precomposed_and_decomposed_form() {
        assert_eq!(fold("Qué FUNCIÓN"), "que funcion");
        assert_eq!(fold("Que\u{301} funcio\u{301}n"), "que funcion");
        assert_eq!(fold("ÑANDÚ"), "nandu");
        assert_eq!(classify_task("la funci\u{f3}n falla"), TaskKind::Code);
    }

    #[test]
    fn the_spanish_word_como_does_not_change_the_task_language() {
        // "como" (I eat / as) must not be read as a code word or an explain prefix by itself.
        assert_eq!(classify_task("como pan todos los días"), TaskKind::General);
        assert_eq!(classify_task("lo hago como puedo"), TaskKind::General);
        assert_eq!(classify_task("como funciona esto"), TaskKind::Explain);
    }
}
