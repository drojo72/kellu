//! The AI switch: when nothing may be sent to a model.
//!
//! It is the first thing a request meets, before the cache, the planner and the router, so "off"
//! costs nothing and leaks nothing. Four things can switch it off, checked in this order:
//!
//! 1. `[ai] enabled = false` in the configuration;
//! 2. the flag file that `kellu ai off` creates (checked on every request, so an editor that is
//!    already running obeys at once, with no restart);
//! 3. the language id of the document, against `[ai] disabled_languages`;
//! 4. the path of the document inside its project, against `[ai] disabled_globs` and the deny
//!    rules.
//!
//! A project can only switch the AI off (see the LSP server), never on. The switch fails closed:
//! a flag file that cannot be examined means off.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use kellu_common::{KelluError, Result};
use kellu_security::DenyGlobs;

use crate::config::AiConfig;

/// File name of the flag, inside the state directory.
pub const FLAG_FILE: &str = "ai-off";

/// Decides whether a request may reach a model.
#[derive(Debug, Clone)]
pub struct AiSwitch {
    enabled: bool,
    languages: Vec<String>,
    globs: DenyGlobs,
    has_globs: bool,
    flag: Option<PathBuf>,
}

impl AiSwitch {
    /// Always on, with no flag file: what a library user gets until they ask for more.
    #[must_use]
    pub fn on() -> Self {
        Self {
            enabled: true,
            languages: Vec::new(),
            globs: DenyGlobs::new(),
            has_globs: false,
            flag: None,
        }
    }

    /// Always off: what to use when the switch cannot be built. Fails closed.
    #[must_use]
    pub fn closed() -> Self {
        Self {
            enabled: false,
            ..Self::on()
        }
    }

    /// The switch described by `config`, and by the flag file at `flag` if there is one.
    ///
    /// # Errors
    /// A glob of the configuration is not valid (the configuration parser already checks this).
    pub fn from_config(config: &AiConfig, flag: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            enabled: config.enabled,
            languages: config.disabled_languages.clone(),
            globs: DenyGlobs::with_user_patterns(&config.disabled_globs)?,
            has_globs: !config.disabled_globs.is_empty(),
            flag,
        })
    }

    /// Whether a request about a document in `language`, at `path` (relative to its project), may
    /// go to a model. Both are optional: the terminal chat has neither.
    ///
    /// # Errors
    /// [`KelluError::AiOff`] with the reason. The reason never repeats the language or the path,
    /// which come from the editor.
    pub fn check(&self, language: Option<&str>, path: Option<&Path>) -> Result<()> {
        if !self.enabled {
            return Err(KelluError::ai_off(
                "AI is off in the configuration ([ai] enabled = false)",
            ));
        }
        self.check_flag()?;
        if let Some(language) = language
            && self
                .languages
                .iter()
                .any(|off| off.eq_ignore_ascii_case(language))
        {
            return Err(KelluError::ai_off(
                "AI is off for this language ([ai] disabled_languages)",
            ));
        }
        if let Some(path) = path
            && self.has_globs
            && self.globs.is_denied(path)
        {
            return Err(KelluError::ai_off(
                "AI is off for this file ([ai] disabled_globs or a deny rule)",
            ));
        }
        Ok(())
    }

    /// Whether the flag file says off. Anything but "not found" counts as off: if the state cannot
    /// be read, the safe answer is that nothing is sent.
    fn check_flag(&self) -> Result<()> {
        let Some(flag) = &self.flag else {
            return Ok(());
        };
        match std::fs::symlink_metadata(flag) {
            Ok(_) => Err(KelluError::ai_off(
                "AI is off (`kellu ai off`); turn it on with `kellu ai on`",
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(_) => Err(KelluError::ai_off(
                "AI is off because its on/off state could not be read",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    fn config(enabled: bool, languages: &[&str], globs: &[&str]) -> AiConfig {
        AiConfig {
            enabled,
            disabled_languages: languages.iter().map(|s| (*s).to_owned()).collect(),
            disabled_globs: globs.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn message(result: Result<()>) -> String {
        match result {
            Err(KelluError::AiOff(text)) => text,
            other => panic!("expected AiOff, got {other:?}"),
        }
    }

    #[test]
    fn it_is_on_by_default() {
        let switch = AiSwitch::on();
        assert!(
            switch
                .check(Some("rust"), Some(Path::new("src/lib.rs")))
                .is_ok()
        );
        assert!(switch.check(None, None).is_ok());
    }

    #[test]
    fn a_closed_switch_never_lets_anything_through() {
        let switch = AiSwitch::closed();
        assert!(switch.check(None, None).is_err());
        assert!(switch.check(Some("rust"), Some(Path::new("a.rs"))).is_err());
    }

    #[test]
    fn the_master_switch_turns_everything_off() {
        let switch = AiSwitch::from_config(&config(false, &[], &[]), None).unwrap();
        assert!(message(switch.check(None, None)).contains("[ai] enabled = false"));
        assert!(switch.check(Some("rust"), Some(Path::new("a.rs"))).is_err());
    }

    #[test]
    fn languages_are_matched_without_regard_to_case() {
        let switch = AiSwitch::from_config(&config(true, &["markdown"], &[]), None).unwrap();
        assert!(switch.check(Some("markdown"), None).is_err());
        assert!(switch.check(Some("Markdown"), None).is_err());
        assert!(switch.check(Some("rust"), None).is_ok());
        assert!(switch.check(None, None).is_ok(), "no language, no match");
    }

    #[test]
    fn globs_switch_off_only_where_they_match() {
        let switch =
            AiSwitch::from_config(&config(true, &[], &["notes/**", "*.txt"]), None).unwrap();
        assert!(switch.check(None, Some(Path::new("notes/a/b.rs"))).is_err());
        assert!(switch.check(None, Some(Path::new("todo.txt"))).is_err());
        assert!(switch.check(None, Some(Path::new("src/lib.rs"))).is_ok());
        assert!(switch.check(None, None).is_ok(), "no path, no match");
    }

    #[test]
    fn the_deny_rules_also_switch_the_ai_off_for_a_file_once_globs_are_in_use() {
        let switch = AiSwitch::from_config(&config(true, &[], &["notes/**"]), None).unwrap();
        assert!(switch.check(None, Some(Path::new(".env"))).is_err());
        assert!(
            switch
                .check(None, Some(Path::new("../outside.rs")))
                .is_err(),
            "a path that climbs out of the project fails closed"
        );
    }

    #[test]
    fn reasons_never_echo_what_the_editor_sent() {
        let switch =
            AiSwitch::from_config(&config(true, &["markdown"], &["secret/**"]), None).unwrap();
        let by_language = message(switch.check(Some("markdown"), None));
        let by_path = message(switch.check(None, Some(Path::new("secret/KEY-123.rs"))));
        assert!(!by_language.contains("markdown"));
        assert!(!by_path.contains("KEY-123") && !by_path.contains("secret/"));
    }

    #[test]
    fn the_flag_file_is_checked_on_every_request() {
        let dir = TempDir::new().unwrap();
        let flag = dir.path().join(FLAG_FILE);
        let switch = AiSwitch::from_config(&config(true, &[], &[]), Some(flag.clone())).unwrap();
        assert!(switch.check(None, None).is_ok());
        fs::write(&flag, "").unwrap();
        assert!(message(switch.check(None, None)).contains("kellu ai on"));
        fs::remove_file(&flag).unwrap();
        assert!(
            switch.check(None, None).is_ok(),
            "turning it on again works at once"
        );
    }

    #[test]
    fn a_flag_that_is_a_symlink_or_dangling_still_means_off() {
        let dir = TempDir::new().unwrap();
        let flag = dir.path().join(FLAG_FILE);
        let switch = AiSwitch::from_config(&config(true, &[], &[]), Some(flag.clone())).unwrap();
        symlink(dir.path().join("nowhere"), &flag).unwrap();
        assert!(switch.check(None, None).is_err());
    }

    #[test]
    fn a_flag_that_cannot_be_examined_fails_closed() {
        let dir = TempDir::new().unwrap();
        // A parent that is a file: the lookup fails with something other than "not found".
        let file = dir.path().join("file");
        fs::write(&file, "").unwrap();
        let flag = file.join(FLAG_FILE);
        let switch = AiSwitch::from_config(&config(true, &[], &[]), Some(flag)).unwrap();
        assert!(message(switch.check(None, None)).contains("could not be read"));
    }

    #[test]
    fn a_missing_state_directory_means_on() {
        let dir = TempDir::new().unwrap();
        let flag = dir.path().join("no/such/dir").join(FLAG_FILE);
        let switch = AiSwitch::from_config(&config(true, &[], &[]), Some(flag)).unwrap();
        assert!(switch.check(None, None).is_ok());
    }
}
