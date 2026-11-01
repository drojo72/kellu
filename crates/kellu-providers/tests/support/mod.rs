//! A scripted HTTP server and builders shared by the provider integration tests.
//!
//! Each reply is served to one connection, then the connection is closed (`Connection: close`),
//! so the client never reuses a socket and the request log is exact.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kellu_providers::{ChatRequest, KeyLookup, Message, ProviderConfig, ProviderKind};
use secrecy::SecretString;

/// The key used by most tests; long enough to be scrubbed from messages.
pub const KEY: &str = "sk-test-key-123456";

/// What the server does with the next connection.
pub enum Reply {
    /// A complete response with a JSON body.
    Json(u16, String),
    /// A complete response with arbitrary bytes.
    Raw(u16, Vec<u8>),
    /// `200` with `text/event-stream`; the body is the given text, then the connection closes.
    Sse(String),
    /// Like `Sse`, but sleeps before closing (an idle or never-ending stream).
    SseStall(String, Duration),
    /// `302 Found` pointing at the given URL.
    Redirect(String),
    /// Read the request, wait, then close without answering.
    Stall(Duration),
    /// Close the connection without answering.
    Drop,
    /// `200` with a `Content-Length` of this many bytes of `a`.
    Big(usize),
}

/// A request as the server saw it.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Recorded {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

pub struct Server {
    port: u16,
    log: Arc<Mutex<Vec<Recorded>>>,
}

fn read_request(stream: &TcpStream) -> Option<Recorded> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let path = first.split_whitespace().nth(1)?.to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Recorded {
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn respond(stream: &mut TcpStream, reply: Reply) {
    let head = |status: u16, extra: &str, length: Option<usize>| {
        let length = length.map_or(String::new(), |n| format!("Content-Length: {n}\r\n"));
        format!("HTTP/1.1 {status} X\r\n{extra}{length}Connection: close\r\n\r\n")
    };
    let _ = match reply {
        Reply::Json(status, body) => {
            let text = head(
                status,
                "Content-Type: application/json\r\n",
                Some(body.len()),
            );
            stream
                .write_all(text.as_bytes())
                .and_then(|()| stream.write_all(body.as_bytes()))
        }
        Reply::Raw(status, body) => {
            let text = head(status, "", Some(body.len()));
            stream
                .write_all(text.as_bytes())
                .and_then(|()| stream.write_all(&body))
        }
        Reply::Sse(body) => {
            let text = head(200, "Content-Type: text/event-stream\r\n", None);
            stream
                .write_all(text.as_bytes())
                .and_then(|()| stream.write_all(body.as_bytes()))
        }
        Reply::SseStall(body, wait) => {
            let text = head(200, "Content-Type: text/event-stream\r\n", None);
            let result = stream
                .write_all(text.as_bytes())
                .and_then(|()| stream.write_all(body.as_bytes()))
                .and_then(|()| stream.flush());
            std::thread::sleep(wait);
            result
        }
        Reply::Redirect(url) => {
            let text = head(302, &format!("Location: {url}\r\n"), Some(0));
            stream.write_all(text.as_bytes())
        }
        Reply::Stall(wait) => {
            std::thread::sleep(wait);
            Ok(())
        }
        Reply::Drop => Ok(()),
        Reply::Big(size) => {
            let text = head(200, "Content-Type: application/json\r\n", Some(size));
            let chunk = vec![b'a'; 64 * 1024];
            let mut result = stream.write_all(text.as_bytes());
            let mut sent = 0;
            while result.is_ok() && sent < size {
                let n = chunk.len().min(size - sent);
                result = stream.write_all(&chunk[..n]);
                sent += n;
            }
            result
        }
    };
    let _ = stream.flush();
}

impl Server {
    /// Serve `replies` in order, one per connection; after the last, answer `500`.
    pub fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&log);
        std::thread::spawn(move || {
            let mut replies = replies.into_iter();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_request(&stream) else {
                    continue;
                };
                shared.lock().unwrap().push(request);
                let reply = replies
                    .next()
                    .unwrap_or_else(|| Reply::Json(500, "{\"error\":\"script exhausted\"}".into()));
                respond(&mut stream, reply);
            }
        });
        Self { port, log }
    }

    /// `http://127.0.0.1:<port>` plus `path`.
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

/// A provider configuration pointing at `base_url`.
pub fn config(
    kind: ProviderKind,
    base_url: &str,
    model: &str,
    keyring: Option<&str>,
) -> ProviderConfig {
    ProviderConfig {
        name: "test".to_owned(),
        kind,
        base_url: base_url.to_owned(),
        model: model.to_owned(),
        timeout_s: 5.0,
        keyring_name: keyring.map(str::to_owned),
    }
}

/// A key lookup that always answers with `value`.
pub fn lookup(value: Option<&str>) -> KeyLookup {
    let value = value.map(str::to_owned);
    Arc::new(move |_| value.clone().map(SecretString::from))
}

/// A one-message user request.
pub fn ask(text: &str) -> ChatRequest {
    ChatRequest::new(vec![Message::user(text)]).unwrap()
}

/// `data: <json>\n\n` for each value, as an SSE body.
pub fn sse(events: &[serde_json::Value]) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}
