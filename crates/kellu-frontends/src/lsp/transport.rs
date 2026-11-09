//! JSON-RPC framing for LSP: `Content-Length` headers and a JSON body.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// Largest body accepted (a whole open document travels in one message).
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
/// Largest header line accepted.
pub const MAX_HEADER_LINE: usize = 1024;
/// Most header lines accepted.
pub const MAX_HEADERS: usize = 16;

/// Why a message could not be read.
#[derive(Debug)]
pub enum TransportError {
    /// The framing is broken; the stream cannot be resynchronized and the server must stop.
    Framing(String),
    /// The body was read in full but is not JSON; the server answers with a parse error and goes on.
    Json,
    /// Reading failed.
    Io(io::Error),
}

fn read_header_line(input: &mut dyn BufRead) -> Result<Option<Vec<u8>>, TransportError> {
    let mut line = Vec::new();
    let limit = u64::try_from(MAX_HEADER_LINE + 1).unwrap_or(u64::MAX);
    Read::take(&mut *input, limit)
        .read_until(b'\n', &mut line)
        .map_err(TransportError::Io)?;
    if line.is_empty() {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return Err(TransportError::Framing(
            "header line too long or cut".into(),
        ));
    }
    Ok(Some(line))
}

/// Read one message. `Ok(None)` is a clean end of input between messages.
///
/// # Errors
/// See [`TransportError`].
pub fn read_message(input: &mut dyn BufRead) -> Result<Option<Value>, TransportError> {
    let mut length: Option<usize> = None;
    let mut headers = 0_usize;
    loop {
        let Some(line) = read_header_line(input)? else {
            return if headers == 0 {
                Ok(None)
            } else {
                Err(TransportError::Framing(
                    "input ended inside the headers".into(),
                ))
            };
        };
        headers += 1;
        if headers > MAX_HEADERS {
            return Err(TransportError::Framing("too many headers".into()));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        let Some((name, value)) = text.split_once(':') else {
            return Err(TransportError::Framing("malformed header".into()));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(TransportError::Framing("duplicate Content-Length".into()));
            }
            // Digits only: `str::parse` would also accept a leading `+`.
            let digits = value.trim();
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(TransportError::Framing("invalid Content-Length".into()));
            }
            let parsed: usize = digits
                .parse()
                .map_err(|_| TransportError::Framing("invalid Content-Length".into()))?;
            if parsed > MAX_BODY_BYTES {
                return Err(TransportError::Framing("message too large".into()));
            }
            length = Some(parsed);
        }
    }
    let Some(length) = length else {
        return Err(TransportError::Framing("missing Content-Length".into()));
    };
    let mut body = vec![0_u8; length];
    input.read_exact(&mut body).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            TransportError::Framing("input ended inside a message".into())
        } else {
            TransportError::Io(error)
        }
    })?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|_| TransportError::Json)
}

/// Write one message.
///
/// # Errors
/// Writing failed (the editor closed the pipe).
pub fn write_message(out: &mut dyn Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn read(bytes: &[u8]) -> Result<Option<Value>, TransportError> {
        read_message(&mut Cursor::new(bytes.to_vec()))
    }

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn a_message_round_trips() {
        let mut wire = Vec::new();
        write_message(&mut wire, &json!({"jsonrpc":"2.0","id":1})).unwrap();
        assert_eq!(read(&wire).unwrap(), Some(json!({"jsonrpc":"2.0","id":1})));
    }

    #[test]
    fn headers_are_case_insensitive_and_other_headers_are_ignored() {
        let body = "{\"a\":1}";
        let wire = format!(
            "content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(read(wire.as_bytes()).unwrap(), Some(json!({"a":1})));
    }

    #[test]
    fn two_messages_back_to_back_and_then_a_clean_end() {
        let mut wire = frame("{\"a\":1}");
        wire.extend(frame("{\"b\":2}"));
        let mut input = Cursor::new(wire);
        assert_eq!(read_message(&mut input).unwrap(), Some(json!({"a":1})));
        assert_eq!(read_message(&mut input).unwrap(), Some(json!({"b":2})));
        assert!(read_message(&mut input).unwrap().is_none());
    }

    #[test]
    fn hostile_framing_is_a_framing_error() {
        for wire in [
            &b"{\"a\":1}"[..],
            b"Content-Length: -1\r\n\r\n",
            b"Content-Length: +5\r\n\r\nxxxxx",
            b"Content-Length: 0x10\r\n\r\n",
            b"Content-Length: 99999999999999999999\r\n\r\n",
            b"Content-Length: 8388609\r\n\r\n",
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            b"Content-Length: 5\r\n\r\n{}",
            b"Content-Length: 5\r\n",
            b"Content-Type: x\r\n\r\n{}",
        ] {
            assert!(
                matches!(read(wire), Err(TransportError::Framing(_))),
                "{:?}",
                String::from_utf8_lossy(wire)
            );
        }
    }

    #[test]
    fn an_endless_header_line_is_cut_off() {
        let mut wire = b"X-Pad: ".to_vec();
        wire.extend(std::iter::repeat_n(b'a', 100_000));
        assert!(matches!(read(&wire), Err(TransportError::Framing(_))));
        let many: String = (0..40).map(|i| format!("X-{i}: v\r\n")).collect();
        assert!(matches!(
            read(many.as_bytes()),
            Err(TransportError::Framing(_))
        ));
    }

    #[test]
    fn a_body_that_is_not_json_is_recoverable() {
        let mut wire = frame("not json");
        wire.extend(frame("{\"ok\":true}"));
        let mut input = Cursor::new(wire);
        assert!(matches!(
            read_message(&mut input),
            Err(TransportError::Json)
        ));
        assert_eq!(read_message(&mut input).unwrap(), Some(json!({"ok":true})));
    }
}
