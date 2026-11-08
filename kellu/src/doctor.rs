//! Environment checks for `kellu doctor`.
//!
//! Tool presence, configuration validity, paths and the sandbox: whether bubblewrap can isolate
//! processes with the exact options Kellu uses, its version against the patched release, whether
//! `prlimit` works inside the sandbox and whether the kernel has `openat2`. Keyring access checks
//! are still to come.

use std::env;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use kellu_common::paths;
use kellu_core::config::Config;
use kellu_sandbox::{FIRST_PATCHED, Probe};

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

/// Probes the sandbox on this host given the paths of `bwrap` and, if found, `prlimit` (injectable
/// so tests never start a real sandbox).
pub type ProbeFn<'a> = &'a dyn Fn(&Path, Option<&Path>) -> Probe;

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

fn config_check(loaded: &kellu_common::Result<Config>) -> Check {
    match loaded {
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

fn sandbox_check(probe: &Probe, require_patched: bool) -> Check {
    let check = |ok: bool, detail: String| Check {
        name: "sandbox",
        ok,
        required: false,
        detail,
    };
    if let Err(reason) = &probe.isolation {
        return check(
            false,
            format!("cannot isolate processes: {reason}; the shell tool stays disabled"),
        );
    }
    let Some(version) = probe.version else {
        return check(
            false,
            "bubblewrap works but its version cannot be read; the shell tool stays disabled"
                .to_owned(),
        );
    };
    if version.is_patched() {
        check(true, format!("bubblewrap {version}, namespaces work"))
    } else if require_patched {
        check(
            false,
            format!(
                "bubblewrap {version} is older than {FIRST_PATCHED} and \
                 sandbox.require_patched_bwrap is true; the shell tool stays disabled"
            ),
        )
    } else {
        check(
            false,
            format!(
                "bubblewrap {version} is older than {FIRST_PATCHED} (GHSA-pxhw-h44j-8pfx, \
                 CVE-2026-87766); allowed because Kellu never creates mount points inside a \
                 project tree. Set sandbox.require_patched_bwrap = true to refuse it"
            ),
        )
    }
}

fn prlimit_check(which: Which<'_>, probe: Option<&Probe>) -> Check {
    let mut check = tool_check(
        which,
        "prlimit",
        false,
        "sandboxed commands cannot run with resource limits, so they are refused",
        "part of util-linux; Debian/Ubuntu: sudo apt install util-linux",
    );
    if check.ok
        && let Some(probe) = probe
        && !probe.prlimit
        && probe.isolation.is_ok()
    {
        check.ok = false;
        check.detail = format!(
            "{} does not work inside the sandbox (it must live under /usr or /bin); \
             sandboxed commands are refused",
            check.detail
        );
    }
    check
}

fn openat2_check(available: bool) -> Check {
    Check {
        name: "openat2",
        ok: available,
        required: false,
        detail: if available {
            "available".to_owned()
        } else {
            "not available (Linux 5.6 or newer is required); reading project files is refused"
                .to_owned()
        },
    }
}

/// Run all checks.
pub fn collect_checks(which: Which<'_>, load: ConfigLoader<'_>, probe: ProbeFn<'_>) -> Vec<Check> {
    let config = load();
    let require_patched = config
        .as_ref()
        .is_ok_and(|config| config.sandbox().require_patched_bwrap);
    let mut checks = vec![
        config_check(&config),
        tool_check(
            which,
            "secret-tool",
            true,
            "API keys cannot be stored in the keyring",
            "Debian/Ubuntu: sudo apt install libsecret-tools",
        ),
        tool_check(
            which,
            "busctl",
            false,
            "`kellu keys list` and `kellu keys cleanup-legacy` cannot list the keyring",
            "part of systemd; Debian/Ubuntu: sudo apt install systemd",
        ),
        tool_check(
            which,
            "bwrap",
            false,
            "the shell tool stays disabled (fail closed)",
            "Debian/Ubuntu: sudo apt install bubblewrap",
        ),
    ];
    let report = which("bwrap").map(|bwrap| probe(&bwrap, which("prlimit").as_deref()));
    if let Some(report) = &report {
        checks.push(sandbox_check(report, require_patched));
    }
    checks.push(prlimit_check(which, report.as_ref()));
    checks.push(openat2_check(
        report
            .as_ref()
            .map_or_else(kellu_security::openat2_available, |report| report.openat2),
    ));
    checks.push(tool_check(
        which,
        "git",
        false,
        "git tools are unavailable",
        "Debian/Ubuntu: sudo apt install git",
    ));
    checks.push(tool_check(
        which,
        "gh",
        false,
        "the pull request tool is unavailable",
        "see https://cli.github.com",
    ));
    checks
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
pub fn run(
    which: Which<'_>,
    load: ConfigLoader<'_>,
    probe: ProbeFn<'_>,
    out: &mut dyn Write,
) -> u8 {
    let checks = collect_checks(which, load, probe);
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
    use kellu_sandbox::BwrapVersion;

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

    fn probe_of(version: Option<(u32, u32, u32)>, isolation: Result<(), &str>) -> Probe {
        Probe {
            version: version.map(|(major, minor, patch)| BwrapVersion {
                major,
                minor,
                patch,
            }),
            isolation: isolation.map_err(str::to_owned),
            prlimit: true,
            openat2: true,
        }
    }

    fn healthy(_: &Path, _: Option<&Path>) -> Probe {
        probe_of(Some((0, 12, 0)), Ok(()))
    }

    fn old(_: &Path, _: Option<&Path>) -> Probe {
        probe_of(Some((0, 9, 0)), Ok(()))
    }

    fn report_with(which: Which<'_>, load: ConfigLoader<'_>, probe: ProbeFn<'_>) -> (u8, String) {
        let mut out = Vec::new();
        let code = run(which, load, probe, &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    fn report(which: Which<'_>, load: ConfigLoader<'_>) -> (u8, String) {
        report_with(which, load, &healthy)
    }

    fn strict() -> kellu_common::Result<Config> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().map_err(|e| KelluError::io(Path::new("."), e))?;
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[sandbox]\nrequire_patched_bwrap = true\n")
            .and_then(|()| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)))
            .map_err(|e| KelluError::io(&path, e))?;
        load_config(Some(&path))
    }

    #[test]
    fn all_tools_present_passes() {
        let (code, text) = report(&everything, &defaults);
        assert_eq!(code, 0);
        assert!(!text.contains("FAIL") && !text.contains("warn"), "{text}");
        assert!(
            text.contains("[  ok] config: packaged defaults (6 providers)"),
            "{text}"
        );
        assert!(
            text.contains("[  ok] sandbox: bubblewrap 0.12.0, namespaces work"),
            "{text}"
        );
        assert!(text.contains("[  ok] openat2"), "{text}");
    }

    #[test]
    fn checks_are_marked_required_or_optional() {
        let checks = collect_checks(&everything, &defaults, &healthy);
        let required: Vec<_> = checks
            .iter()
            .filter(|c| c.required)
            .map(|c| c.name)
            .collect();
        assert_eq!(required, ["config", "secret-tool"]);
        for name in ["bwrap", "sandbox", "prlimit", "openat2"] {
            assert!(
                checks.iter().any(|c| c.name == name && !c.required),
                "{name}"
            );
        }
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
        // Without bubblewrap there is nothing to probe.
        assert!(!text.contains("sandbox:"), "{text}");
    }

    #[test]
    fn nothing_is_probed_when_bubblewrap_is_missing() {
        let probe = |_: &Path, _: Option<&Path>| -> Probe {
            unreachable!("the sandbox must not be probed without bwrap")
        };
        let (code, _) = report_with(&only(&["secret-tool"]), &defaults, &probe);
        assert_eq!(code, 0);
    }

    #[test]
    fn an_old_bubblewrap_is_a_warning_that_names_the_advisory_and_the_switch() {
        let (code, text) = report_with(&everything, &defaults, &old);
        assert_eq!(code, 0, "{text}");
        let line = text.lines().find(|l| l.contains("sandbox:")).unwrap();
        assert!(line.starts_with("[warn]"), "{line}");
        for needle in [
            "0.9.0",
            "older than 0.12.0",
            "GHSA-pxhw-h44j-8pfx",
            "sandbox.require_patched_bwrap = true",
        ] {
            assert!(line.contains(needle), "{needle}: {line}");
        }
    }

    #[test]
    fn strict_mode_says_the_shell_tool_stays_disabled_for_an_old_bubblewrap() {
        let (code, text) = report_with(&everything, &strict, &old);
        assert_eq!(code, 0, "{text}");
        let line = text.lines().find(|l| l.contains("sandbox:")).unwrap();
        assert!(line.starts_with("[warn]"), "{line}");
        assert!(line.contains("require_patched_bwrap is true"), "{line}");
        assert!(line.contains("stays disabled"), "{line}");
        // A patched bubblewrap is fine in strict mode.
        let (_, text) = report_with(&everything, &strict, &healthy);
        assert!(text.contains("[  ok] sandbox"), "{text}");
    }

    #[test]
    fn a_sandbox_that_cannot_isolate_says_why_and_does_not_fail_the_doctor() {
        let blocked = |_: &Path, _: Option<&Path>| {
            probe_of(
                Some((0, 12, 0)),
                Err("bwrap: No permissions to creating new namespace"),
            )
        };
        let (code, text) = report_with(&everything, &defaults, &blocked);
        assert_eq!(code, 0, "{text}");
        let line = text.lines().find(|l| l.contains("sandbox:")).unwrap();
        assert!(line.starts_with("[warn]"), "{line}");
        assert!(line.contains("No permissions to creating new namespace"));
        assert!(line.contains("stays disabled"));
    }

    #[test]
    fn an_unreadable_version_is_a_warning() {
        let unknown = |_: &Path, _: Option<&Path>| probe_of(None, Ok(()));
        let (_, text) = report_with(&everything, &defaults, &unknown);
        let line = text.lines().find(|l| l.contains("sandbox:")).unwrap();
        assert!(line.starts_with("[warn]") && line.contains("version cannot be read"));
    }

    #[test]
    fn missing_openat2_and_unusable_prlimit_are_reported() {
        let degraded = |_: &Path, _: Option<&Path>| Probe {
            prlimit: false,
            openat2: false,
            ..probe_of(Some((0, 12, 0)), Ok(()))
        };
        let (code, text) = report_with(&everything, &defaults, &degraded);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("[warn] openat2: not available"), "{text}");
        assert!(
            text.contains("[warn] prlimit") && text.contains("does not work inside the sandbox"),
            "{text}"
        );
    }

    #[test]
    fn a_missing_prlimit_is_reported_with_its_consequence() {
        let (_, text) = report(&only(&["secret-tool", "bwrap"]), &defaults);
        let line = text.lines().find(|l| l.contains("prlimit")).unwrap();
        assert!(line.starts_with("[warn]"), "{line}");
        assert!(line.contains("resource limits"), "{line}");
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
