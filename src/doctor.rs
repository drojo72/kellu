//! Environment checks for `kellu doctor`.
//!
//! Basic so far: tool presence, configuration validity and paths. Sandbox probing (user
//! namespaces, bubblewrap version) and keyring access checks arrive in M4.

use std::env;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use kellu_common::paths;
use kellu_core::config::Config;

/// Result of one environment check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub required: bool,
    pub detail: String,
}

/// Looks up an executable by name (injectable so tests never depend on the host).
pub type Which<'a> = &'a dyn Fn(&str) -> Option<PathBuf>;

/// Loads the configuration (injectable for the same reason).
pub type ConfigLoader<'a> = &'a dyn Fn() -> kellu_common::Result<Config>;

/// Find `binary` in `PATH`: a regular file with at least one execute bit.
pub fn find_in_path(binary: &str) -> Option<PathBuf> {
    if binary.contains('/') {
        return None;
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.mode() & 0o111 != 0)
}

fn tool_check(
    which: Which<'_>,
    binary: &'static str,
    required: bool,
    purpose: &str,
    hint: &str,
) -> Check {
    let found = which(binary);
    let detail = match &found {
        Some(path) => path.display().to_string(),
        None => format!("not found; {purpose}. {hint}"),
    };
    Check {
        name: binary,
        ok: found.is_some(),
        required,
        detail,
    }
}

fn config_check(load: ConfigLoader<'_>) -> Check {
    match load() {
        Ok(config) => {
            let place = config.source().map_or_else(
                || "packaged defaults".to_owned(),
                |p| p.display().to_string(),
            );
            Check {
                name: "config",
                ok: true,
                required: true,
                detail: format!("{place} ({} providers)", config.providers().len()),
            }
        }
        Err(error) => Check {
            name: "config",
            ok: false,
            required: true,
            detail: error.to_string(),
        },
    }
}

/// Run all checks.
pub fn collect_checks(which: Which<'_>, load: ConfigLoader<'_>) -> Vec<Check> {
    vec![
        config_check(load),
        tool_check(
            which,
            "secret-tool",
            true,
            "API keys cannot be stored in the keyring",
            "Debian/Ubuntu: sudo apt install libsecret-tools",
        ),
        tool_check(
            which,
            "bwrap",
            false,
            "the shell tool stays disabled (fail closed)",
            "Debian/Ubuntu: sudo apt install bubblewrap",
        ),
        tool_check(
            which,
            "git",
            false,
            "git tools are unavailable",
            "Debian/Ubuntu: sudo apt install git",
        ),
    ]
}

fn say(out: &mut dyn Write, text: &str) {
    let _ = writeln!(out, "{text}");
}

fn show_dir(label: &str, dir: kellu_common::Result<PathBuf>) -> String {
    match dir {
        Ok(path) => format!("{label} {}", path.display()),
        Err(error) => format!("{label} unknown ({error})"),
    }
}

/// Print a report and return the exit code (0 when every required check passes).
pub fn run(which: Which<'_>, load: ConfigLoader<'_>, out: &mut dyn Write) -> u8 {
    let checks = collect_checks(which, load);
    for check in &checks {
        let status = match (check.ok, check.required) {
            (true, _) => "ok",
            (false, true) => "FAIL",
            (false, false) => "warn",
        };
        say(
            out,
            &format!("[{status:>4}] {}: {}", check.name, check.detail),
        );
    }
    say(out, "");
    say(out, &show_dir("config:", paths::config_dir()));
    say(out, &show_dir("state: ", paths::state_dir()));
    say(out, &show_dir("cache: ", paths::cache_dir()));
    say(out, "");
    say(
        out,
        "Sandbox probing (user namespaces, bubblewrap policy) arrives in M4.",
    );
    if checks.iter().all(|c| c.ok || !c.required) {
        crate::cli::EXIT_OK
    } else {
        crate::cli::EXIT_ERROR
    }
}

#[cfg(test)]
mod tests {
    use kellu_common::KelluError;
    use kellu_core::config::load_config;

    use super::*;

    fn everything(name: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/usr/bin").join(name))
    }

    fn only(allowed: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |name| {
            allowed
                .contains(&name)
                .then(|| PathBuf::from("/usr/bin").join(name))
        }
    }

    fn defaults() -> kellu_common::Result<Config> {
        // A path that does not exist: packaged defaults only, independent of the host.
        load_config(Some(Path::new("/nonexistent/kellu-test/config.toml")))
    }

    fn broken() -> kellu_common::Result<Config> {
        Err(KelluError::config(
            "providers: at least one provider is required",
        ))
    }

    fn report(which: Which<'_>, load: ConfigLoader<'_>) -> (u8, String) {
        let mut out = Vec::new();
        let code = run(which, load, &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn all_tools_present_passes() {
        let (code, text) = report(&everything, &defaults);
        assert_eq!(code, 0);
        assert!(!text.contains("FAIL") && !text.contains("warn"), "{text}");
        assert!(
            text.contains("[  ok] config: packaged defaults (5 providers)"),
            "{text}"
        );
    }

    #[test]
    fn checks_are_marked_required_or_optional() {
        let checks = collect_checks(&everything, &defaults);
        let required: Vec<_> = checks
            .iter()
            .filter(|c| c.required)
            .map(|c| c.name)
            .collect();
        assert_eq!(required, ["config", "secret-tool"]);
        assert!(checks.iter().any(|c| c.name == "bwrap" && !c.required));
    }

    #[test]
    fn missing_required_tool_fails() {
        let (code, text) = report(&only(&["bwrap", "git"]), &defaults);
        assert_eq!(code, 1);
        assert!(text.contains("[FAIL] secret-tool"));
        assert!(text.contains("libsecret-tools"));
    }

    #[test]
    fn missing_optional_tool_only_warns() {
        let (code, text) = report(&only(&["secret-tool"]), &defaults);
        assert_eq!(code, 0);
        assert!(text.contains("[warn] bwrap"));
        assert!(text.contains("fail closed"));
    }

    #[test]
    fn invalid_config_fails_the_check() {
        let (code, text) = report(&everything, &broken);
        assert_eq!(code, 1);
        assert!(
            text.contains("[FAIL] config: providers: at least one provider is required"),
            "{text}"
        );
    }

    #[test]
    fn find_in_path_rejects_names_with_slashes() {
        assert_eq!(find_in_path("/bin/sh"), None);
        assert_eq!(find_in_path("../sh"), None);
    }
}
