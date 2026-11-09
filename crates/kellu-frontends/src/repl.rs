//! The terminal chat: one line in, one streamed answer out.
//!
//! The REPL never touches files, tools or the network itself. Text goes to the model only through
//! [`Assistant::chat`] (redacted and bounded), and everything that comes back is stripped of
//! control characters before it reaches the terminal, so a model cannot move the cursor, rewrite
//! the screen or set the window title.

use std::io::{BufRead, Read, Write};

use kellu_common::text::strip_control_chars;
use kellu_core::assist::Assistant;
use kellu_core::session::{EntryKind, SessionLog};
use kellu_providers::types::{Message, StreamEvent};

use crate::theme::{Role, Theme};

/// Longest line accepted, in bytes. A longer paste is dropped whole, never cut in the middle.
pub const MAX_LINE_BYTES: usize = 16 * 1024;
/// Messages kept in memory (older ones are dropped first).
pub const MAX_MESSAGES: usize = 100;

const HELP: &str = "/help   this list\n/clear  forget the conversation\n/exit   leave (Ctrl-D too)";

/// The terminal chat.
pub struct Repl<'a> {
    assistant: &'a Assistant,
    theme: Theme,
    session: Option<SessionLog>,
    history: Vec<Message>,
}

enum Line {
    Text(String),
    TooLong,
    Eof,
}

/// Read one line of at most [`MAX_LINE_BYTES`]; an over-long line is consumed up to its newline.
fn read_line(input: &mut dyn BufRead) -> std::io::Result<Line> {
    let mut buffer = Vec::new();
    let limit = u64::try_from(MAX_LINE_BYTES + 1).unwrap_or(u64::MAX);
    Read::take(&mut *input, limit).read_until(b'\n', &mut buffer)?;
    if buffer.is_empty() {
        return Ok(Line::Eof);
    }
    if buffer.last() != Some(&b'\n') && buffer.len() > MAX_LINE_BYTES {
        // Drain the rest of the line without keeping it.
        loop {
            let chunk = input.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            match chunk.iter().position(|&b| b == b'\n') {
                Some(at) => {
                    input.consume(at + 1);
                    break;
                }
                None => {
                    let all = chunk.len();
                    input.consume(all);
                }
            }
        }
        return Ok(Line::TooLong);
    }
    let text = String::from_utf8_lossy(&buffer);
    Ok(Line::Text(text.trim_end_matches(['\n', '\r']).to_owned()))
}

impl<'a> Repl<'a> {
    /// A chat over `assistant`. `session`, if any, records the conversation (it redacts by itself).
    #[must_use]
    pub fn new(assistant: &'a Assistant, theme: Theme, session: Option<SessionLog>) -> Self {
        Self {
            assistant,
            theme,
            session,
            history: Vec::new(),
        }
    }

    fn record(&mut self, kind: EntryKind, text: &str, err: &mut dyn Write) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if let Err(error) = session.log(kind, text, None) {
            // A broken log must not break the chat, but the user should know it stopped.
            let _ = writeln!(err, "note: session log stopped: {error}");
            self.session = None;
        }
    }

    fn push(&mut self, message: Message) {
        self.history.push(message);
        let extra = self.history.len().saturating_sub(MAX_MESSAGES);
        if extra > 0 {
            self.history.drain(..extra);
        }
    }

    /// Run until end of input or `/exit`. Returns the process exit code (always 0: leaving is not
    /// a failure, and a failed answer is reported and the loop goes on).
    pub fn run(&mut self, input: &mut dyn BufRead, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
        if let Some(Err(error)) = self.session.as_mut().map(SessionLog::start) {
            let _ = writeln!(err, "note: session log disabled: {error}");
            self.session = None;
        }
        let _ = writeln!(out, "{} Kellu. /help for commands.", self.theme.mark());
        loop {
            let _ = write!(out, "{}", self.theme.prompt());
            let _ = out.flush();
            let line = match read_line(input) {
                Ok(Line::Eof) | Err(_) => break,
                Ok(Line::TooLong) => {
                    self.problem(err, "line too long; ignored");
                    continue;
                }
                Ok(Line::Text(text)) => text,
            };
            let trimmed = line.trim();
            match trimmed {
                "" => {}
                "/exit" | "/quit" => break,
                "/help" => {
                    let _ = writeln!(out, "{HELP}");
                }
                "/clear" => {
                    self.history.clear();
                    let _ = writeln!(
                        out,
                        "{}",
                        self.theme.paint(Role::Note, "conversation cleared")
                    );
                }
                command if command.starts_with('/') => {
                    let name = command.split_whitespace().next().unwrap_or("/");
                    let message =
                        format!("unknown command {}; try /help", strip_control_chars(name));
                    self.problem(err, &message);
                }
                text => self.ask(text, out, err),
            }
        }
        if let Some(session) = self.session.as_mut() {
            let _ = session.end(&[]);
        }
        let _ = writeln!(out);
        0
    }

    fn problem(&self, err: &mut dyn Write, message: &str) {
        let _ = writeln!(
            err,
            "{}",
            self.theme.paint(Role::Error, &format!("error: {message}"))
        );
    }

    fn ask(&mut self, text: &str, out: &mut dyn Write, err: &mut dyn Write) {
        let text = strip_control_chars(text);
        self.record(EntryKind::User, &text, err);
        self.push(Message::user(text));
        let mut answer = String::new();
        let failure = match self.assistant.chat(&self.history) {
            Err(error) => Some(error.to_string()),
            Ok(stream) => {
                let mut failure = None;
                for event in stream {
                    match event {
                        Ok(StreamEvent::Text(delta)) => {
                            let clean = strip_control_chars(&delta);
                            answer.push_str(&clean);
                            let _ = write!(out, "{clean}");
                            let _ = out.flush();
                        }
                        Ok(StreamEvent::Done(_)) => {}
                        Err(error) => {
                            failure = Some(error.to_string());
                            break;
                        }
                    }
                }
                failure
            }
        };
        if let Some(reason) = failure {
            let reason = strip_control_chars(&reason);
            self.record(EntryKind::Error, &reason, err);
            // Keep what the user already saw only when something was delivered.
            if answer.is_empty() {
                self.history.pop();
            } else {
                self.push(Message::assistant(answer));
            }
            let _ = writeln!(out);
            self.problem(err, &reason);
            return;
        }
        let _ = writeln!(out);
        self.record(EntryKind::Assistant, &answer, err);
        self.push(Message::assistant(answer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read_all(bytes: &[u8]) -> Vec<String> {
        let mut input = Cursor::new(bytes.to_vec());
        let mut lines = Vec::new();
        loop {
            match read_line(&mut input).unwrap_or(Line::Eof) {
                Line::Text(t) => lines.push(t),
                Line::TooLong => lines.push("<long>".to_owned()),
                Line::Eof => break,
            }
        }
        lines
    }

    #[test]
    fn lines_are_split_and_the_last_one_needs_no_newline() {
        assert_eq!(read_all(b"a\r\nb\nc"), ["a", "b", "c"]);
        assert!(read_all(b"").is_empty());
    }

    #[test]
    fn an_over_long_line_is_dropped_whole_and_the_next_one_survives() {
        let mut data = vec![b'x'; MAX_LINE_BYTES * 3];
        data.extend_from_slice(b"\nnext\n");
        assert_eq!(read_all(&data), ["<long>", "next"]);
        let mut exact = vec![b'y'; MAX_LINE_BYTES];
        exact.push(b'\n');
        assert_eq!(read_all(&exact).len(), 1);
        assert_ne!(
            read_all(&exact),
            ["<long>"],
            "a line of exactly the limit is accepted"
        );
    }

    #[test]
    fn invalid_utf8_is_replaced_not_fatal() {
        let lines = read_all(b"ok \xff\xfe end\n");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{fffd}'));
    }
}
