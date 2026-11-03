//! Reading a secret from the user: standard input only, hidden on a terminal.
//!
//! Secrets are never accepted as command-line arguments (they would show up in `ps` and in the
//! shell history). On a terminal the prompt goes to stderr and echo is turned off for the read;
//! echo is restored even if the read fails. When stdin is a pipe, the first line is the secret.

use std::io::{BufRead, Read, Write};

use kellu_common::{KelluError, Result};
use rustix::termios::{self, LocalModes, OptionalActions, Termios};
use secrecy::SecretString;
use zeroize::{Zeroize, Zeroizing};

/// Largest secret accepted (the vault enforces the same limit).
pub const MAX_SECRET_BYTES: usize = kellu_security::store::MAX_SECRET_BYTES;

/// Read one line as a secret from `reader`, without keeping stray copies of it.
///
/// A trailing `\n` or `\r\n` is removed. At most [`MAX_SECRET_BYTES`] are read.
///
/// # Errors
/// The line is empty, too long or not UTF-8, or reading failed.
pub fn read_secret_line(reader: &mut dyn BufRead) -> Result<SecretString> {
    let limit = (MAX_SECRET_BYTES + 2) as u64;
    let mut line = Zeroizing::new(Vec::with_capacity(MAX_SECRET_BYTES + 2));
    reader
        .take(limit)
        .read_until(b'\n', &mut line)
        .map_err(|e| KelluError::io(std::path::Path::new("<stdin>"), e))?;
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    if line.len() > MAX_SECRET_BYTES {
        return Err(KelluError::config("the secret is too long"));
    }
    if line.is_empty() {
        return Err(KelluError::config(
            "no secret was provided on standard input",
        ));
    }
    match String::from_utf8(std::mem::take(&mut *line)) {
        Ok(text) => Ok(SecretString::from(text)),
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            Err(KelluError::config("the secret is not valid UTF-8"))
        }
    }
}

/// Restores terminal settings when dropped.
struct EchoGuard {
    original: Termios,
}

impl Drop for EchoGuard {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(
            rustix::stdio::stdin(),
            OptionalActions::Flush,
            &self.original,
        );
    }
}

/// Read the secret from the process's standard input. On a terminal the input is hidden and
/// `prompt` is shown on `err`.
///
/// # Errors
/// As [`read_secret_line`], or the terminal could not be configured.
pub fn read_secret(prompt: &str, err: &mut dyn Write) -> Result<SecretString> {
    let stdin = rustix::stdio::stdin();
    let stdin_path = std::path::Path::new("<stdin>");
    if !termios::isatty(stdin) {
        return read_secret_line(&mut std::io::stdin().lock());
    }
    let original = termios::tcgetattr(stdin).map_err(|e| KelluError::io(stdin_path, e.into()))?;
    let mut silent = original.clone();
    silent.local_modes.remove(LocalModes::ECHO);
    termios::tcsetattr(stdin, OptionalActions::Flush, &silent)
        .map_err(|e| KelluError::io(stdin_path, e.into()))?;
    let _guard = EchoGuard { original };
    let _ = write!(err, "{prompt}");
    let _ = err.flush();
    let result = read_secret_line(&mut std::io::stdin().lock());
    let _ = writeln!(err);
    result
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    fn read(bytes: &[u8]) -> Result<SecretString> {
        read_secret_line(&mut std::io::Cursor::new(bytes.to_vec()))
    }

    #[test]
    fn the_first_line_is_the_secret() {
        assert_eq!(read(b"sk-abc123\n").unwrap().expose_secret(), "sk-abc123");
        assert_eq!(read(b"sk-abc123\r\n").unwrap().expose_secret(), "sk-abc123");
        assert_eq!(read(b"sk-abc123").unwrap().expose_secret(), "sk-abc123");
        assert_eq!(read(b"first\nsecond\n").unwrap().expose_secret(), "first");
    }

    #[test]
    fn nothing_or_a_blank_line_is_an_error() {
        assert!(read(b"").is_err());
        assert!(read(b"\n").is_err());
        assert!(read(b"\r\n").is_err());
    }

    #[test]
    fn oversized_input_is_refused_without_reading_it_all() {
        let mut huge = vec![b'a'; MAX_SECRET_BYTES * 4];
        huge.push(b'\n');
        let mut cursor = std::io::Cursor::new(huge);
        assert!(read_secret_line(&mut cursor).is_err());
        assert!(cursor.position() <= (MAX_SECRET_BYTES + 2) as u64);
    }

    #[test]
    fn a_secret_exactly_at_the_limit_is_accepted() {
        let mut input = vec![b'a'; MAX_SECRET_BYTES];
        input.push(b'\n');
        assert_eq!(
            read(&input).unwrap().expose_secret().len(),
            MAX_SECRET_BYTES
        );
    }

    #[test]
    fn invalid_utf8_is_refused() {
        assert!(read(b"\xff\xfe\xfd\n").is_err());
    }

    #[test]
    fn errors_never_contain_the_input() {
        let error = read(b"\xff sk-secret-value-123\n").unwrap_err().to_string();
        assert!(!error.contains("secret-value"), "{error}");
    }
}
