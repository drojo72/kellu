//! `file://` URIs to absolute paths, strictly.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = hex(*bytes.get(i + 1)?)?;
            let low = hex(*bytes.get(i + 2)?)?;
            out.push(high << 4 | low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The absolute path a `file:` URI names, or `None` for anything else: another scheme, a remote
/// host, a query or fragment, a NUL byte, a relative path, or a `..` component (after decoding,
/// so `%2e%2e` is caught too).
#[must_use]
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let (scheme, rest) = uri.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let rest = rest.strip_prefix("//")?;
    let slash = rest.find('/')?;
    let (host, encoded) = rest.split_at(slash);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    if encoded.contains(['?', '#']) {
        return None;
    }
    let bytes = percent_decode(encoded)?;
    if bytes.contains(&0) {
        return None;
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    let clean = path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)));
    clean.then_some(path)
}

/// Whether `path` is absolute with only normal components (used for roots given as `rootPath`).
#[must_use]
pub fn is_clean_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_encoded_paths_decode() {
        assert_eq!(
            path_from_uri("file:///home/u/a.rs"),
            Some("/home/u/a.rs".into())
        );
        assert_eq!(
            path_from_uri("file:///home/u/my%20dir/%C3%A9.rs"),
            Some("/home/u/my dir/\u{e9}.rs".into())
        );
        assert_eq!(
            path_from_uri("FILE://localhost/tmp/x"),
            Some("/tmp/x".into())
        );
        assert_eq!(path_from_uri("file:///a%23b"), Some("/a#b".into()));
    }

    #[test]
    fn everything_else_is_refused() {
        for uri in [
            "http://example.com/a",
            "untitled:Untitled-1",
            "file://evil.example/etc/passwd",
            "file:relative/path",
            "file:///a/../b",
            "file:///a/%2e%2e/b",
            "file:///a/%2E%2E",
            "file:///a%00b",
            "file:///a?x=1",
            "file:///a#frag",
            "file:///a%zz",
            "file:///a%4",
            "file://",
            "",
            "not a uri",
        ] {
            assert_eq!(path_from_uri(uri), None, "{uri}");
        }
    }

    #[test]
    fn dot_segments_and_double_slashes_are_normal_or_refused() {
        assert_eq!(path_from_uri("file:///a/./b"), Some("/a/b".into()));
        assert_eq!(path_from_uri("file:///a//b"), Some("/a/b".into()));
    }
}
