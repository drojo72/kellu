//! Deny globs: paths that are never read, listed or sent to a provider.
//!
//! The built-in rules cannot be removed; users may only add. A path that is not a plain
//! relative path (absolute, `..`, not UTF-8) is denied: the check fails closed.
//!
//! Pattern language (deliberately tiny): `*` matches any run of characters and `?` one
//! character, inside a single path component. A pattern **without** `/` matches any component
//! of the path (`*.pem` denies `a/b/key.pem`). A pattern **with** `/` is matched from the start
//! of the path, component by component (`secrets/*` denies `secrets/a/b`). Matching ignores
//! ASCII case, so `.ENV` does not slip through.

use std::path::{Component, Path};

use kellu_common::{KelluError, Result};

/// Component patterns that are always denied.
const BUILTIN: &[&str] = &[
    ".env*",
    "*.pem",
    "*.key",
    "id_*",
    "credentials*",
    "*.token",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    "*.kdbx",
    "*.secret",
    "*.secrets",
    "secrets.*",
    ".netrc",
    ".npmrc",
    ".pypirc",
];

/// Hidden names that stay readable. Every other hidden file or directory is denied, which also
/// covers `.git/` internals, `.ssh/`, `.gnupg/` and `.aws/`.
const HIDDEN_ALLOWED: &[&str] = &[".gitignore", ".editorconfig", ".github", ".kateproject"];

const MAX_PATTERN_LEN: usize = 128;
const MAX_USER_PATTERNS: usize = 256;

/// The deny rules in force: the built-in ones plus the user's additions.
#[derive(Debug, Clone, Default)]
pub struct DenyGlobs {
    user: Vec<String>,
}

impl DenyGlobs {
    /// Only the built-in rules.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in rules plus `patterns`.
    ///
    /// # Errors
    /// A pattern is empty, too long, contains a NUL or a backslash, is absolute or contains `..`.
    pub fn with_user_patterns<I, S>(patterns: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut user = Vec::new();
        for pattern in patterns {
            let pattern = pattern.as_ref();
            let bad = pattern.is_empty()
                || pattern.len() > MAX_PATTERN_LEN
                || pattern.contains(['\0', '\\'])
                || pattern.starts_with('/')
                || pattern.split('/').any(|part| part == "..");
            if bad {
                return Err(KelluError::config(
                    "deny pattern: expected a relative glob (no `..`, `\\` or NUL)",
                ));
            }
            user.push(pattern.trim_end_matches('/').to_ascii_lowercase());
        }
        if user.len() > MAX_USER_PATTERNS {
            return Err(KelluError::config("deny patterns: too many"));
        }
        Ok(Self { user })
    }

    /// Whether `relative` (a path inside a project) must not be read or sent.
    #[must_use]
    pub fn is_denied(&self, relative: &Path) -> bool {
        let Some(parts) = components(relative) else {
            return true;
        };
        if parts.is_empty() {
            return true;
        }
        let hidden = parts.iter().any(|part| {
            part.starts_with('.')
                && !HIDDEN_ALLOWED
                    .iter()
                    .any(|ok| ok.eq_ignore_ascii_case(part))
        });
        if hidden
            || parts
                .iter()
                .any(|part| BUILTIN.iter().any(|g| glob(g, part)))
        {
            return true;
        }
        self.user.iter().any(|pattern| {
            if pattern.contains('/') {
                let segments: Vec<&str> = pattern.split('/').collect();
                segments.len() <= parts.len()
                    && segments.iter().zip(&parts).all(|(g, p)| glob(g, p))
            } else {
                parts.iter().any(|part| glob(pattern, part))
            }
        })
    }
}

/// The normal components of `path` as strings, or `None` when it is not a plain relative path.
fn components(path: &Path) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_str()?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts)
}

/// `*` and `?` matching on one component, ignoring ASCII case. Iterative, so a hostile name
/// cannot cause exponential backtracking.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi].eq_ignore_ascii_case(&t[ti])) && p[pi] != b'*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((star_p, star_t)) = star {
            pi = star_p + 1;
            ti = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|byte| *byte == b'*')
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    fn denied(path: &str) -> bool {
        DenyGlobs::new().is_denied(Path::new(path))
    }

    #[test]
    fn secrets_and_keys_are_denied_wherever_they_are() {
        for path in [
            ".env",
            ".env.local",
            "app/.env",
            "config/.env.production",
            "server.pem",
            "a/b/tls.key",
            "id_rsa",
            "home/id_ed25519",
            "credentials.json",
            "aws/credentials",
            "ci.token",
            "store.kdbx",
            "cert.p12",
            "app/secrets.yaml",
            "prod.secret",
            ".netrc",
            ".npmrc",
        ] {
            assert!(denied(path), "{path}");
        }
    }

    #[test]
    fn case_does_not_matter() {
        for path in [
            ".ENV",
            "KEY.PEM",
            "Id_Rsa",
            "CREDENTIALS.JSON",
            ".Git/config",
        ] {
            assert!(denied(path), "{path}");
        }
    }

    #[test]
    fn hidden_entries_are_denied_except_the_allowlist() {
        for path in [
            ".git/config",
            ".git/hooks/pre-commit",
            ".ssh/known_hosts",
            ".gnupg/pubring.kbx",
            ".aws/config",
            ".cache/x",
            "a/.hidden/b",
        ] {
            assert!(denied(path), "{path}");
        }
        for path in [
            ".gitignore",
            ".editorconfig",
            ".github/workflows/ci.yml",
            ".kateproject",
            "sub/.gitignore",
        ] {
            assert!(!denied(path), "{path}");
        }
        assert!(denied(".github/.secrets-dir/x"));
    }

    #[test]
    fn ordinary_source_files_are_allowed() {
        for path in [
            "src/main.rs",
            "README.md",
            "Cargo.toml",
            "docs/design/lsp.md",
            "crates/kellu-security/src/lib.rs",
            "./src/lib.rs",
        ] {
            assert!(!denied(path), "{path}");
        }
    }

    #[test]
    fn paths_that_are_not_plain_relative_paths_fail_closed() {
        for path in ["", ".", "/etc/passwd", "../x", "a/../b", "a/../../b", "/"] {
            assert!(denied(path), "{path:?}");
        }
        let non_utf8 = Path::new(OsStr::from_bytes(b"caf\xe9.txt"));
        assert!(DenyGlobs::new().is_denied(non_utf8));
    }

    #[test]
    fn users_can_add_patterns_but_not_remove_built_ins() {
        let globs =
            DenyGlobs::with_user_patterns(["*.sql", "vendor/", "private/*/data", "Dump?.txt"])
                .unwrap();
        for path in [
            "db/schema.sql",
            "vendor/lib/x.rs",
            "private/a/data/x",
            "dump1.txt",
        ] {
            assert!(globs.is_denied(Path::new(path)), "{path}");
        }
        for path in ["src/main.rs", "private/a/other", "vendors/x.rs"] {
            assert!(!globs.is_denied(Path::new(path)), "{path}");
        }
        assert!(globs.is_denied(Path::new(".env")));
    }

    #[test]
    fn bad_user_patterns_are_rejected() {
        for bad in [
            "",
            "/abs",
            "../up",
            "a/../b",
            "a\\b",
            "a\0b",
            &"x".repeat(200),
        ] {
            assert!(DenyGlobs::with_user_patterns([bad]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_matcher_handles_stars_and_questions() {
        assert!(glob("*", ""));
        assert!(glob("a*b*c", "aXXbYYc"));
        assert!(glob("a?c", "abc"));
        assert!(!glob("a?c", "ac"));
        assert!(!glob("a*b", "ac"));
        assert!(glob("**x", "abcx"));
        assert!(!glob("abc", "abcd"));
    }

    #[test]
    fn a_hostile_name_cannot_make_the_matcher_explode() {
        let name = "a".repeat(5000);
        let globs = DenyGlobs::with_user_patterns(["*a*a*a*a*a*a*a*a*a*b"]).unwrap();
        let started = std::time::Instant::now();
        assert!(!globs.is_denied(Path::new(&name)));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
