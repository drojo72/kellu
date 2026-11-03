//! Scrubbed environments for child processes.
//!
//! A child process gets an **allowlist** of variables, never the full environment: tokens
//! exported in a shell profile must not leak into `secret-tool` today or into sandboxed tools
//! tomorrow. Variables whose names look like secrets are dropped even if they were allowlisted.

use std::ffi::OsString;

/// Longest value passed through.
const MAX_VALUE_BYTES: usize = 4096;

/// What `secret-tool` needs to reach the user's Secret Service over D-Bus.
pub const KEYRING_ALLOWLIST: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "HOME",
    "LANG",
    "LC_ALL",
];

/// A conservative set for tools that run on the user's behalf (locale, terminal, time zone).
pub const TOOL_ALLOWLIST: &[&str] = &["LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ"];

const SECRET_WORDS: &[&str] = &[
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "AUTH",
];

fn looks_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_WORDS.iter().any(|word| upper.contains(word))
}

/// Keep only the variables of `source` named in `allow`, sorted by name.
///
/// Dropped: names not in the allowlist, names that look like secrets, values that are not UTF-8,
/// contain NUL or are longer than 4 KiB.
#[must_use]
pub fn scrub<I>(allow: &[&str], source: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut kept: Vec<(String, String)> = source
        .into_iter()
        .filter_map(|(name, value)| {
            let (name, value) = (name.into_string().ok()?, value.into_string().ok()?);
            let allowed = allow.contains(&name.as_str())
                && !looks_secret(&name)
                && value.len() <= MAX_VALUE_BYTES
                && !value.contains('\0');
            allowed.then_some((name, value))
        })
        .collect();
    kept.sort();
    kept.dedup_by(|a, b| a.0 == b.0);
    kept
}

/// [`scrub`] applied to this process's environment.
#[must_use]
pub fn scrubbed_from_process(allow: &[&str]) -> Vec<(String, String)> {
    scrub(allow, std::env::vars_os())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[test]
    fn only_allowlisted_variables_survive() {
        let source = vars(&[
            ("HOME", "/home/u"),
            ("GITHUB_TOKEN", "ghp_x"),
            ("AWS_SECRET_ACCESS_KEY", "x"),
            ("LANG", "C.UTF-8"),
            ("ANYTHING_ELSE", "1"),
        ]);
        let kept = scrub(KEYRING_ALLOWLIST, source);
        assert_eq!(
            kept,
            vec![
                ("HOME".to_owned(), "/home/u".to_owned()),
                ("LANG".to_owned(), "C.UTF-8".to_owned())
            ]
        );
    }

    #[test]
    fn secret_looking_names_are_dropped_even_if_allowlisted() {
        let kept = scrub(
            &["MY_API_KEY", "HOME"],
            vars(&[("MY_API_KEY", "x"), ("HOME", "/h")]),
        );
        assert_eq!(kept, vec![("HOME".to_owned(), "/h".to_owned())]);
    }

    #[test]
    fn hostile_values_are_dropped() {
        let long = "x".repeat(MAX_VALUE_BYTES + 1);
        let kept = scrub(
            &["HOME", "LANG", "TERM"],
            vars(&[("HOME", "/h\0x"), ("LANG", &long), ("TERM", "xterm")]),
        );
        assert_eq!(kept, vec![("TERM".to_owned(), "xterm".to_owned())]);
    }

    #[test]
    fn non_utf8_values_are_dropped() {
        use std::os::unix::ffi::OsStringExt;
        let source = vec![(OsString::from("HOME"), OsString::from_vec(vec![0xff, 0xfe]))];
        assert!(scrub(&["HOME"], source).is_empty());
    }

    #[test]
    fn the_allowlists_contain_no_secret_looking_names() {
        for name in KEYRING_ALLOWLIST.iter().chain(TOOL_ALLOWLIST) {
            assert!(!looks_secret(name), "{name}");
        }
    }
}
