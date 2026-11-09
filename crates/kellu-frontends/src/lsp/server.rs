//! The LSP state machine. [`Server::handle`] is pure over its input (it returns the messages to
//! send), so the whole protocol is testable without pipes.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use kellu_common::text::strip_control_chars;
use kellu_core::assist::{Assistant, Snippet};
use kellu_security::{DenyGlobs, Scope};
use serde_json::{Value, json};

use super::text::{offset_at, position_at, word_at};
use super::transport::{TransportError, read_message, write_message};
use super::uri::{is_clean_absolute, path_from_uri};

/// Documents kept at once.
pub const MAX_DOCUMENTS: usize = 64;
/// Largest document kept, in bytes. A bigger one is not tracked (and so never sent anywhere).
pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
/// Answers remembered.
const CACHE_ENTRIES: usize = 128;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const SERVER_NOT_INITIALIZED: i64 = -32002;

/// Which model features are on. The configuration sets them; a project can only turn them off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    /// Explain the symbol under the cursor.
    pub hover: bool,
    /// Complete code at the cursor.
    pub completion: bool,
}

/// What the loop should do after a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep reading.
    Continue,
    /// Stop with this process exit code.
    Exit(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Uninitialized,
    Running,
    ShuttingDown,
}

#[derive(Debug)]
struct Document {
    text: String,
    version: i64,
    language: Option<String>,
}

type CacheKey = (&'static str, String, i64, usize);

/// A Kate LSP server over an [`Assistant`].
pub struct Server<'a> {
    assistant: &'a Assistant,
    configured: Features,
    features: Features,
    home: PathBuf,
    state: State,
    scope: Option<Scope>,
    documents: BTreeMap<String, Document>,
    cache: BTreeMap<CacheKey, Option<Value>>,
}

fn response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn valid_id(id: &Value) -> bool {
    id.is_number() || id.is_string()
}

impl<'a> Server<'a> {
    /// A server that has not been initialized. `home` is the user's home directory (a project root
    /// equal to it, to one of its ancestors or to `/` is refused).
    #[must_use]
    pub fn new(assistant: &'a Assistant, features: Features, home: PathBuf) -> Self {
        Self {
            assistant,
            configured: features,
            features,
            home,
            state: State::Uninitialized,
            scope: None,
            documents: BTreeMap::new(),
            cache: BTreeMap::new(),
        }
    }

    /// Read messages from `input` and answer on `out` until `exit` or the end of input. Returns the
    /// process exit code: 0 after `shutdown` then `exit`, 1 otherwise.
    pub fn serve(&mut self, input: &mut dyn BufRead, out: &mut dyn Write) -> u8 {
        loop {
            let (replies, flow) = match read_message(input) {
                Ok(Some(message)) => self.handle(&message),
                Ok(None) => return 1,
                Err(TransportError::Json) => (
                    vec![failure(
                        &Value::Null,
                        PARSE_ERROR,
                        "the body is not valid JSON",
                    )],
                    Flow::Continue,
                ),
                Err(TransportError::Framing(reason)) => {
                    log::warn!("lsp: {reason}");
                    return 1;
                }
                Err(TransportError::Io(_)) => return 1,
            };
            for reply in &replies {
                if write_message(out, reply).is_err() {
                    return 1;
                }
            }
            if let Flow::Exit(code) = flow {
                return code;
            }
        }
    }

    /// Handle one message and return what to send back.
    pub fn handle(&mut self, message: &Value) -> (Vec<Value>, Flow) {
        let Some(object) = message.as_object() else {
            // Batches and scalars: LSP has none.
            return (
                vec![failure(
                    &Value::Null,
                    INVALID_REQUEST,
                    "expected a JSON-RPC object",
                )],
                Flow::Continue,
            );
        };
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            // A response to a request we never sent, or junk: ignore.
            return (Vec::new(), Flow::Continue);
        };
        let params = object.get("params").unwrap_or(&Value::Null);
        match object.get("id") {
            None => (Vec::new(), self.notification(method, params)),
            Some(id) if valid_id(id) => (vec![self.request(id, method, params)], Flow::Continue),
            Some(_) => (
                vec![failure(&Value::Null, INVALID_REQUEST, "invalid request id")],
                Flow::Continue,
            ),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) -> Flow {
        match method {
            "exit" => return Flow::Exit(u8::from(self.state != State::ShuttingDown)),
            "textDocument/didOpen" if self.state == State::Running => self.did_open(params),
            "textDocument/didChange" if self.state == State::Running => self.did_change(params),
            "textDocument/didClose" if self.state == State::Running => {
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    self.documents.remove(uri);
                    self.cache.retain(|key, _| key.1 != uri);
                }
            }
            // `initialized`, `$/cancelRequest`, `workspace/*`, anything unknown: nothing to do.
            _ => {}
        }
        Flow::Continue
    }

    fn request(&mut self, id: &Value, method: &str, params: &Value) -> Value {
        match (self.state, method) {
            (State::Uninitialized, "initialize") => self.initialize(id, params),
            (State::Uninitialized, _) => {
                failure(id, SERVER_NOT_INITIALIZED, "the server is not initialized")
            }
            (State::ShuttingDown, _) => failure(id, INVALID_REQUEST, "the server is shutting down"),
            (State::Running, "initialize") => {
                failure(id, INVALID_REQUEST, "the server is already initialized")
            }
            (State::Running, "shutdown") => {
                self.state = State::ShuttingDown;
                self.documents.clear();
                self.cache.clear();
                response(id, Value::Null)
            }
            (State::Running, "textDocument/hover") if self.features.hover => {
                response(id, self.hover(params))
            }
            (State::Running, "textDocument/completion") if self.features.completion => {
                response(id, self.completion(params))
            }
            (State::Running, _) => failure(id, METHOD_NOT_FOUND, "method not supported"),
        }
    }

    fn initialize(&mut self, id: &Value, params: &Value) -> Value {
        self.state = State::Running;
        self.scope = None;
        let root = params
            .get("rootUri")
            .and_then(Value::as_str)
            .and_then(path_from_uri)
            .or_else(|| {
                params
                    .pointer("/workspaceFolders/0/uri")
                    .and_then(Value::as_str)
                    .and_then(path_from_uri)
            })
            .or_else(|| {
                params
                    .get("rootPath")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .filter(|path| is_clean_absolute(path))
            });
        match root {
            Some(root) => match Scope::open(&root, &self.home, DenyGlobs::new(), false) {
                Ok(scope) => self.scope = Some(scope),
                Err(error) => log::warn!("lsp: project root refused: {error}"),
            },
            None => log::warn!("lsp: no usable project root"),
        }
        // Settings from the project (`.kateproject`) are untrusted: they can switch a feature off,
        // never on.
        let options = params.pointer("/initializationOptions/kellu");
        let allowed = |name: &str, configured: bool| {
            configured
                && options
                    .and_then(|o| o.get(name))
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
        };
        self.features = Features {
            hover: allowed("hover", self.configured.hover),
            completion: allowed("completion", self.configured.completion),
        };
        let mut capabilities = json!({
            // Full-text sync: simple, and Kate sends whole documents anyway.
            "textDocumentSync": {"openClose": true, "change": 1},
        });
        if self.features.hover {
            capabilities["hoverProvider"] = json!(true);
        }
        if self.features.completion {
            // No trigger characters: only an explicit completion request calls the model.
            capabilities["completionProvider"] = json!({});
        }
        response(
            id,
            json!({
                "capabilities": capabilities,
                "serverInfo": {"name": "kellu", "version": env!("CARGO_PKG_VERSION")},
            }),
        )
    }

    /// The path of `uri` relative to the project root if the document may be used, else `None`.
    ///
    /// Fails closed: no usable root, a path outside the root (also through a symlinked directory),
    /// a denied name, or a file that is itself a symlink or not a regular file all say no. A file
    /// that does not exist yet (an unsaved buffer) is allowed when its directory is.
    fn permitted(&self, uri: &str) -> Option<PathBuf> {
        let scope = self.scope.as_ref()?;
        let path = path_from_uri(uri)?;
        let name = path.file_name()?;
        let parent = fs::canonicalize(path.parent()?).ok()?;
        let full = parent.join(name);
        let relative = full.strip_prefix(scope.root()).ok()?.to_path_buf();
        if relative.as_os_str().is_empty() || scope.is_denied(&relative) {
            return None;
        }
        match fs::symlink_metadata(&full) {
            Ok(_) => scope.open_file(&relative).ok().map(|_| relative),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let dir = relative.parent().unwrap_or(Path::new(""));
                scope.open_dir(dir).ok().map(|_| relative)
            }
            Err(_) => None,
        }
    }

    fn store(&mut self, uri: &str, text: &str, version: i64, language: Option<&str>) {
        let fits = text.len() <= MAX_DOCUMENT_BYTES
            && (self.documents.contains_key(uri) || self.documents.len() < MAX_DOCUMENTS);
        // Denied or oversized documents are not kept at all, and an older copy is dropped.
        if !fits || self.permitted(uri).is_none() {
            self.documents.remove(uri);
            self.cache.retain(|key, _| key.1 != uri);
            return;
        }
        self.documents.insert(
            uri.to_owned(),
            Document {
                text: text.to_owned(),
                version,
                language: language.map(str::to_owned),
            },
        );
    }

    fn did_open(&mut self, params: &Value) {
        let document = params.get("textDocument");
        let (Some(uri), Some(text)) = (
            document.and_then(|d| d.get("uri")).and_then(Value::as_str),
            document.and_then(|d| d.get("text")).and_then(Value::as_str),
        ) else {
            return;
        };
        let version = document
            .and_then(|d| d.get("version"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let language = document
            .and_then(|d| d.get("languageId"))
            .and_then(Value::as_str);
        self.store(uri, text, version, language);
    }

    fn did_change(&mut self, params: &Value) {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return;
        };
        let Some(old) = self.documents.get(uri) else {
            return;
        };
        let version = params
            .pointer("/textDocument/version")
            .and_then(Value::as_i64)
            .unwrap_or(old.version);
        let language = old.language.clone();
        // With full sync every change carries the whole text; one with a range means the client
        // ignored our setting, and applying it blindly could leave a wrong copy: drop the document.
        let whole = params
            .get("contentChanges")
            .and_then(Value::as_array)
            .and_then(|changes| changes.last())
            .filter(|change| change.get("range").is_none())
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str);
        match whole {
            Some(text) if version >= old.version => {
                self.store(uri, text, version, language.as_deref());
            }
            _ => {
                self.documents.remove(uri);
            }
        }
    }

    /// The document, its byte offset and the identifier, for a position request that is allowed.
    fn target(&self, params: &Value) -> Option<(&Document, String, usize)> {
        let uri = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)?;
        let line = params.pointer("/position/line").and_then(Value::as_u64)?;
        let character = params
            .pointer("/position/character")
            .and_then(Value::as_u64)?;
        // The document is checked again now: the file may have been replaced by a link since.
        self.permitted(uri)?;
        let document = self.documents.get(uri)?;
        let offset = offset_at(&document.text, line, character)?;
        Some((document, uri.to_owned(), offset))
    }

    fn remember(&mut self, key: CacheKey, value: Option<Value>) {
        if self.cache.len() >= CACHE_ENTRIES {
            self.cache.clear();
        }
        self.cache.insert(key, value.clone());
    }

    fn hover(&mut self, params: &Value) -> Value {
        let Some((document, uri, offset)) = self.target(params) else {
            return Value::Null;
        };
        let (start, end) = word_at(&document.text, offset);
        if start == end {
            return Value::Null;
        }
        let key: CacheKey = ("hover", uri, document.version, start);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone().unwrap_or(Value::Null);
        }
        let snippet = Snippet {
            language: document.language.as_deref(),
            before: &document.text[..start],
            after: &document.text[end..],
        };
        let focus = &document.text[start..end];
        let result = match self.assistant.explain(&snippet, focus) {
            Ok(text) if !text.trim().is_empty() => {
                let (first_line, first_char) = position_at(&document.text, start);
                let (last_line, last_char) = position_at(&document.text, end);
                Some(json!({
                    // Plain text: a model's markdown could carry links or images.
                    "contents": {"kind": "plaintext", "value": text},
                    "range": {
                        "start": {"line": first_line, "character": first_char},
                        "end": {"line": last_line, "character": last_char},
                    },
                }))
            }
            Ok(_) => None,
            Err(error) => {
                log::warn!(
                    "lsp: hover failed: {}",
                    strip_control_chars(&error.to_string())
                );
                return Value::Null;
            }
        };
        self.remember(key, result.clone());
        result.unwrap_or(Value::Null)
    }

    fn completion(&mut self, params: &Value) -> Value {
        let empty = || json!({"isIncomplete": false, "items": []});
        let Some((document, uri, offset)) = self.target(params) else {
            return empty();
        };
        let key: CacheKey = ("completion", uri, document.version, offset);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone().unwrap_or_else(empty);
        }
        let snippet = Snippet {
            language: document.language.as_deref(),
            before: &document.text[..offset],
            after: &document.text[offset..],
        };
        let (line, character) = position_at(&document.text, offset);
        let (word_start, _) = word_at(&document.text, offset);
        let typed = document.text[word_start..offset].to_owned();
        let result = match self.assistant.complete(&snippet) {
            Ok(text) if !text.trim().is_empty() => {
                let label: String = text
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or("Kellu")
                    .chars()
                    .take(60)
                    .collect();
                let position = json!({"line": line, "character": character});
                let mut item = json!({
                    "label": label,
                    "kind": 1,
                    "detail": "Kellu",
                    // Plain text: never let the model's `$1` or `${...}` become snippet syntax.
                    "insertTextFormat": 1,
                    "textEdit": {
                        "range": {"start": position, "end": position},
                        "newText": text,
                    },
                });
                if !typed.is_empty() {
                    // The client filters by what is typed; keep the suggestion visible.
                    item["filterText"] = json!(typed);
                }
                Some(json!({"isIncomplete": false, "items": [item]}))
            }
            Ok(_) => None,
            Err(error) => {
                log::warn!(
                    "lsp: completion failed: {}",
                    strip_control_chars(&error.to_string())
                );
                return empty();
            }
        };
        self.remember(key, result.clone());
        result.unwrap_or_else(empty)
    }
}
