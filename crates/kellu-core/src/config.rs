//! Configuration: packaged defaults merged with an optional user `config.toml`.
//!
//! Design rules (see SECURITY.md, T11):
//!
//! * Configuration is data, never code, and never holds secrets. Keys that look like secrets are
//!   rejected, with a pointer to the keyring.
//! * A tampered configuration could redirect API traffic, so the user file must be a regular file
//!   owned by the current user and not writable by group or others, endpoints must use HTTPS
//!   (plain HTTP only for localhost), and unknown keys are errors rather than silently ignored.
//! * Loading has no side effects: nothing is created, and there are no global singletons.
//! * The TOML is read as a plain table and validated by hand, so every error names the exact key
//!   and never echoes a value (a secret pasted into the wrong place must not end up in an error).

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};

use kellu_common::net::endpoint_problem;
use kellu_common::text::{strip_control_chars, truncate};
use kellu_common::{KelluError, Result, paths};
use kellu_providers::types::{ProviderConfig, ProviderKind};
use toml::{Table, Value};

use crate::log::{LogLevel, LogSettings};

/// File name of the user configuration inside the config directory.
pub const CONFIG_FILENAME: &str = "config.toml";
/// Largest user configuration file that is read.
pub const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Route used for task types that have no route of their own.
pub const FALLBACK_ROUTE: &str = "general";

const DEFAULTS: &str = include_str!("defaults.toml");

const TOP_KEYS: &[&str] = &["providers", "routing", "router", "logging", "sessions"];
const PROVIDER_KEYS: &[&str] = &["kind", "base_url", "model", "timeout_s", "keyring_name"];
const ROUTE_KEYS: &[&str] = &["primary", "backup"];
const ROUTER_KEYS: &[&str] = &["total_timeout_s"];
const LOGGING_KEYS: &[&str] = &["level", "max_bytes", "backups"];
const SESSIONS_KEYS: &[&str] = &["enabled", "max_entry_chars"];

const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "token",
    "secret",
    "password",
    "passwd",
    "authorization",
    "bearer",
];
const SECRET_SUFFIXES: &[&str] = &["_api_key", "_apikey", "_token", "_secret", "_password"];

/// Ordered provider chain for a task type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    primary: String,
    backup: Vec<String>,
}

impl Route {
    /// The provider tried first.
    #[must_use]
    pub fn primary(&self) -> &str {
        &self.primary
    }

    /// Providers to try, in order, after the primary one.
    #[must_use]
    pub fn backup(&self) -> &[String] {
        &self.backup
    }

    /// Providers to try, in order: the primary one, then the backups.
    pub fn chain(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.primary.as_str()).chain(self.backup.iter().map(String::as_str))
    }
}

/// Limits for the provider router.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterConfig {
    /// Total time budget for one request across all providers, in seconds.
    pub total_timeout_s: f64,
}

/// Session log settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionsConfig {
    /// Whether sessions are written at all.
    pub enabled: bool,
    /// Longest entry written, in characters.
    pub max_entry_chars: usize,
}

/// Validated, immutable configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    providers: BTreeMap<String, ProviderConfig>,
    routing: BTreeMap<String, Route>,
    fallback: Route,
    router: RouterConfig,
    logging: LogSettings,
    sessions: SessionsConfig,
    source: Option<PathBuf>,
}

impl Config {
    /// Providers by name.
    #[must_use]
    pub fn providers(&self) -> &BTreeMap<String, ProviderConfig> {
        &self.providers
    }

    /// Routes by task type.
    #[must_use]
    pub fn routing(&self) -> &BTreeMap<String, Route> {
        &self.routing
    }

    /// The route for `task`, or the `general` route when the task has none.
    #[must_use]
    pub fn route(&self, task: &str) -> &Route {
        self.routing.get(task).unwrap_or(&self.fallback)
    }

    /// Router limits.
    #[must_use]
    pub fn router(&self) -> &RouterConfig {
        &self.router
    }

    /// Application log settings.
    #[must_use]
    pub fn logging(&self) -> &LogSettings {
        &self.logging
    }

    /// Session log settings.
    #[must_use]
    pub fn sessions(&self) -> &SessionsConfig {
        &self.sessions
    }

    /// The user file that was applied, if there was one.
    #[must_use]
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }
}

// --------------------------------------------------------------------- helpers ----

/// Make a key or name from the file safe to show: no control characters, bounded length.
fn show(text: &str) -> String {
    let limit = std::num::NonZeroUsize::MIN.saturating_add(39);
    strip_control_chars(&truncate(text, limit))
}

fn error_at(location: &str, message: impl Display) -> KelluError {
    KelluError::config(format!("{location}: {message}"))
}

fn as_table<'a>(value: &'a Value, location: &str) -> Result<&'a Table> {
    value
        .as_table()
        .ok_or_else(|| error_at(location, "expected a table"))
}

fn section<'a>(table: &'a Table, key: &str) -> Result<Option<&'a Table>> {
    table.get(key).map(|v| as_table(v, key)).transpose()
}

fn check_keys(table: &Table, allowed: &[&str], location: &str) -> Result<()> {
    let mut unknown: Vec<String> = table
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .map(|key| show(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort();
    Err(error_at(
        location,
        format!("unknown key(s): {}", unknown.join(", ")),
    ))
}

fn is_secret_key(key: &str) -> bool {
    let low = key.to_ascii_lowercase();
    SECRET_KEYS.contains(&low.as_str()) || SECRET_SUFFIXES.iter().any(|s| low.ends_with(s))
}

fn reject_secret_keys(value: &Value, location: &str) -> Result<()> {
    match value {
        Value::Table(table) => {
            for (key, inner) in table {
                let here = format!("{location}.{}", show(key));
                if is_secret_key(key) {
                    return Err(error_at(
                        &here,
                        "secrets must not be stored in the configuration; \
                         use `kellu keys set` to store them in the keyring",
                    ));
                }
                reject_secret_keys(inner, &here)?;
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                reject_secret_keys(item, &format!("{location}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn deep_merge(base: &Table, over: &Table) -> Table {
    let mut merged = base.clone();
    for (key, right) in over {
        let value = match (merged.get(key), right) {
            (Some(Value::Table(left)), Value::Table(right)) => {
                Value::Table(deep_merge(left, right))
            }
            _ => right.clone(),
        };
        merged.insert(key.clone(), value);
    }
    merged
}

fn get_str<'a>(table: &'a Table, key: &str, location: &str) -> Result<&'a str> {
    match table.get(key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text),
        _ => Err(error_at(
            &format!("{location}.{key}"),
            "expected a non-empty string",
        )),
    }
}

fn get_int(table: &Table, key: &str, location: &str, low: i64, high: i64) -> Result<i64> {
    match table.get(key) {
        Some(Value::Integer(n)) if (low..=high).contains(n) => Ok(*n),
        _ => Err(error_at(
            &format!("{location}.{key}"),
            format!("expected an integer between {low} and {high}"),
        )),
    }
}

fn get_number(table: &Table, key: &str, location: &str, low: f64, high: f64) -> Result<f64> {
    let number = match table.get(key) {
        Some(Value::Integer(n)) => Some(*n as f64),
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    };
    number.filter(|n| (low..=high).contains(n)).ok_or_else(|| {
        error_at(
            &format!("{location}.{key}"),
            format!("expected a number between {low} and {high}"),
        )
    })
}

fn get_bool(table: &Table, key: &str, location: &str) -> Result<bool> {
    match table.get(key) {
        Some(Value::Boolean(flag)) => Ok(*flag),
        _ => Err(error_at(
            &format!("{location}.{key}"),
            "expected true or false",
        )),
    }
}

/// `^[a-z][a-z0-9_-]{0,31}$`
fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn check_name<'a>(name: &'a str, location: &str) -> Result<&'a str> {
    if is_valid_name(name) {
        Ok(name)
    } else {
        Err(error_at(
            location,
            format!(
                "invalid name {:?} (use a-z, 0-9, _ or -; max 32)",
                show(name)
            ),
        ))
    }
}

// --------------------------------------------------------------------- parsing ----

fn parse_provider(name: &str, raw: &Value) -> Result<ProviderConfig> {
    let location = format!("providers.{}", check_name(name, "providers")?);
    let table = as_table(raw, &location)?;
    check_keys(table, PROVIDER_KEYS, &location)?;

    let kind_name = get_str(table, "kind", &location)?;
    let kind = ProviderKind::parse(kind_name).ok_or_else(|| {
        let names: Vec<_> = ProviderKind::ALL.iter().map(|k| k.as_str()).collect();
        error_at(
            &format!("{location}.kind"),
            format!("must be one of {}", names.join(", ")),
        )
    })?;
    let timeout_s = get_number(table, "timeout_s", &location, 1.0, 300.0)?;
    let base_url = get_str(table, "base_url", &location)?;
    if let Some(problem) = endpoint_problem(base_url) {
        return Err(error_at(&format!("{location}.base_url"), problem));
    }
    let keyring_name = match table.get("keyring_name") {
        None => None,
        Some(Value::String(value)) => {
            Some(check_name(value, &format!("{location}.keyring_name"))?.to_owned())
        }
        Some(_) => {
            return Err(error_at(
                &format!("{location}.keyring_name"),
                "expected a string",
            ));
        }
    };
    Ok(ProviderConfig {
        name: name.to_owned(),
        kind,
        base_url: base_url.trim_end_matches('/').to_owned(),
        model: get_str(table, "model", &location)?.to_owned(),
        timeout_s,
        keyring_name,
    })
}

fn parse_route(
    name: &str,
    raw: &Value,
    providers: &BTreeMap<String, ProviderConfig>,
) -> Result<Route> {
    let location = format!("routing.{}", check_name(name, "routing")?);
    let table = as_table(raw, &location)?;
    check_keys(table, ROUTE_KEYS, &location)?;
    let primary = get_str(table, "primary", &location)?.to_owned();
    let backup = match table.get("backup") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                error_at(
                    &format!("{location}.backup"),
                    "expected a list of provider names",
                )
            })?,
        Some(_) => {
            return Err(error_at(
                &format!("{location}.backup"),
                "expected a list of provider names",
            ));
        }
    };
    let route = Route { primary, backup };
    let unknown: Vec<String> = route
        .chain()
        .filter(|item| !providers.contains_key(*item))
        .map(show)
        .collect();
    if !unknown.is_empty() {
        return Err(error_at(
            &location,
            format!("unknown provider(s): {}", unknown.join(", ")),
        ));
    }
    let mut seen: Vec<&str> = route.chain().collect();
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != 1 + route.backup.len() {
        return Err(error_at(
            &location,
            "a provider appears more than once in the chain",
        ));
    }
    Ok(route)
}

/// Merge `user` over `defaults` and validate the result. Pure: no filesystem access.
///
/// # Errors
///
/// [`KelluError::Config`] naming the offending key when the merged configuration is invalid.
pub fn parse_config(
    defaults: &Table,
    user: Option<&Table>,
    source: Option<PathBuf>,
) -> Result<Config> {
    let merged = match user {
        Some(user) => deep_merge(defaults, user),
        None => defaults.clone(),
    };
    reject_secret_keys(&Value::Table(merged.clone()), "config")?;
    check_keys(&merged, TOP_KEYS, "config")?;

    let empty = Table::new();

    let providers = section(&merged, "providers")?
        .unwrap_or(&empty)
        .iter()
        .map(|(name, raw)| Ok((name.clone(), parse_provider(name, raw)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    if providers.is_empty() {
        return Err(error_at("providers", "at least one provider is required"));
    }

    let routing = section(&merged, "routing")?
        .unwrap_or(&empty)
        .iter()
        .map(|(name, raw)| Ok((name.clone(), parse_route(name, raw, &providers)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let fallback = routing.get(FALLBACK_ROUTE).cloned().ok_or_else(|| {
        error_at(
            &format!("routing.{FALLBACK_ROUTE}"),
            "the fallback route is required",
        )
    })?;

    let router_table = section(&merged, "router")?.unwrap_or(&empty);
    check_keys(router_table, ROUTER_KEYS, "router")?;
    let router = RouterConfig {
        total_timeout_s: get_number(router_table, "total_timeout_s", "router", 5.0, 900.0)?,
    };

    let log_table = section(&merged, "logging")?.unwrap_or(&empty);
    check_keys(log_table, LOGGING_KEYS, "logging")?;
    let level = LogLevel::parse(get_str(log_table, "level", "logging")?).ok_or_else(|| {
        error_at(
            "logging.level",
            format!("must be one of {}", LogLevel::NAMES.join(", ")),
        )
    })?;
    let logging = LogSettings {
        level,
        max_bytes: get_int(log_table, "max_bytes", "logging", 10_000, 100_000_000)?.unsigned_abs(),
        backups: u32::try_from(get_int(log_table, "backups", "logging", 0, 20)?).unwrap_or(0),
    };

    let sessions_table = section(&merged, "sessions")?.unwrap_or(&empty);
    check_keys(sessions_table, SESSIONS_KEYS, "sessions")?;
    let sessions = SessionsConfig {
        enabled: get_bool(sessions_table, "enabled", "sessions")?,
        max_entry_chars: usize::try_from(get_int(
            sessions_table,
            "max_entry_chars",
            "sessions",
            100,
            100_000,
        )?)
        .unwrap_or(100),
    };

    Ok(Config {
        providers,
        routing,
        fallback,
        router,
        logging,
        sessions,
        source,
    })
}

// --------------------------------------------------------------------- loading ----

/// Location of the user configuration file (it may not exist).
///
/// # Errors
///
/// [`KelluError::Config`] when the home directory cannot be determined.
pub fn user_config_path() -> Result<PathBuf> {
    Ok(paths::config_dir()?.join(CONFIG_FILENAME))
}

/// Read the packaged defaults.
///
/// # Errors
///
/// [`KelluError::Config`] if the embedded file is not valid TOML (a build defect).
pub fn load_defaults() -> Result<Table> {
    parse_toml(DEFAULTS).map_err(|e| KelluError::config(format!("packaged defaults: {e}")))
}

/// Parse TOML text into a table. The error names the line, never the content.
fn parse_toml(text: &str) -> std::result::Result<Table, String> {
    text.parse::<Table>().map_err(|e| {
        let line = e.span().map_or(0, |span| {
            text.as_bytes()[..span.start.min(text.len())]
                .iter()
                .filter(|&&b| b == b'\n')
                .count()
                + 1
        });
        let message = strip_control_chars(e.message());
        if line == 0 {
            format!("invalid TOML ({message})")
        } else {
            format!("invalid TOML at line {line} ({message})")
        }
    })
}

fn read_user_file(path: &Path) -> Result<Option<Table>> {
    let Some(bytes) = paths::read_trusted_file(path, MAX_CONFIG_BYTES)? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes)
        .map_err(|_| KelluError::config(format!("{}: not valid UTF-8", path.display())))?;
    parse_toml(&text)
        .map(Some)
        .map_err(|e| KelluError::config(format!("{}: {e}", path.display())))
}

/// Load the packaged defaults plus the user file (default: `~/.config/kellu/config.toml`).
///
/// Creates nothing on disk.
///
/// # Errors
///
/// [`KelluError::Config`] when the content is invalid; [`KelluError::Security`] when the user
/// file is unsafe (symlink, foreign owner, group or world writable).
pub fn load_config(user_file: Option<&Path>) -> Result<Config> {
    let path = match user_file {
        Some(path) => path.to_path_buf(),
        None => user_config_path()?,
    };
    let user = read_user_file(&path)?;
    let source = user.is_some().then_some(path);
    parse_config(&load_defaults()?, user.as_ref(), source)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use tempfile::TempDir;

    use super::*;

    /// Defaults plus the given user TOML, parsed (no filesystem).
    fn with(user: &str) -> Result<Config> {
        let defaults = load_defaults().unwrap();
        let user: Table = user.parse().unwrap();
        parse_config(&defaults, Some(&user), None)
    }

    fn message(result: Result<Config>) -> String {
        result.unwrap_err().to_string()
    }

    fn write_user_file(dir: &TempDir, text: &str, mode: u32) -> PathBuf {
        let path = dir.path().join("config.toml");
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    // ----- defaults -----

    #[test]
    fn packaged_defaults_are_valid() {
        let config = parse_config(&load_defaults().unwrap(), None, None).unwrap();
        let names: Vec<_> = config.providers().keys().map(String::as_str).collect();
        assert_eq!(
            names,
            ["deepseek", "gemma_direct", "google", "grok", "openrouter"]
        );
        assert_eq!(config.routing().len(), 3);
        assert_eq!(config.router().total_timeout_s, 120.0);
        assert_eq!(config.logging(), &LogSettings::default());
        assert_eq!(
            config.sessions(),
            &SessionsConfig {
                enabled: true,
                max_entry_chars: 2000
            }
        );
        assert_eq!(config.source(), None);
    }

    #[test]
    fn default_routes_match_the_design() {
        let config = parse_config(&load_defaults().unwrap(), None, None).unwrap();
        let chain = |task: &str| config.route(task).chain().collect::<Vec<_>>();
        assert_eq!(
            chain("general"),
            ["gemma_direct", "google", "deepseek", "openrouter"]
        );
        assert_eq!(chain("code"), ["deepseek", "openrouter"]);
        assert_eq!(chain("explain"), ["google", "openrouter"]);
    }

    #[test]
    fn provider_kinds_match_the_registry() {
        for kind in ProviderKind::ALL {
            let user = format!(
                "[providers.extra]\nkind = \"{}\"\nbase_url = \"https://example.com\"\n\
                 model = \"m\"\ntimeout_s = 5\n",
                kind.as_str()
            );
            let config = with(&user).unwrap();
            assert_eq!(config.providers()["extra"].kind, kind);
        }
        let bad = "[providers.extra]\nkind = \"carrier_pigeon\"\nbase_url = \"https://e.com\"\nmodel = \"m\"\ntimeout_s = 5\n";
        assert!(message(with(bad)).contains("must be one of google, openai_compat"));
    }

    #[test]
    fn trailing_slashes_are_removed_from_base_urls() {
        let config = with("[providers.grok]\nbase_url = \"https://api.x.ai/v1//\"\n").unwrap();
        assert_eq!(config.providers()["grok"].base_url, "https://api.x.ai/v1");
    }

    // ----- overrides -----

    #[test]
    fn user_file_overrides_part_of_a_provider() {
        let config = with("[providers.deepseek]\nmodel = \"deepseek-reasoner\"\n").unwrap();
        let provider = &config.providers()["deepseek"];
        assert_eq!(provider.model, "deepseek-reasoner");
        assert_eq!(provider.base_url, "https://api.deepseek.com/v1");
        assert_eq!(provider.keyring_name.as_deref(), Some("deepseek"));
    }

    #[test]
    fn user_can_add_provider_and_route() {
        let user = "[providers.local]\nkind = \"openai_compat\"\n\
                    base_url = \"http://localhost:11434/v1\"\nmodel = \"llama\"\ntimeout_s = 120\n\
                    [routing.chat]\nprimary = \"local\"\nbackup = [\"deepseek\"]\n";
        let config = with(user).unwrap();
        assert_eq!(config.providers()["local"].keyring_name, None);
        assert_eq!(
            config.route("chat").chain().collect::<Vec<_>>(),
            ["local", "deepseek"]
        );
    }

    #[test]
    fn unknown_task_falls_back_to_general_route() {
        let config = parse_config(&load_defaults().unwrap(), None, None).unwrap();
        assert_eq!(config.route("poetry"), config.route(FALLBACK_ROUTE));
    }

    // ----- strictness -----

    #[test]
    fn unknown_top_level_key_is_rejected() {
        assert!(message(with("mystery = 1\n")).contains("config: unknown key(s): mystery"));
    }

    #[test]
    fn unknown_provider_field_is_rejected() {
        let text = message(with("[providers.grok]\nproxy = \"x\"\n"));
        assert!(
            text.contains("providers.grok: unknown key(s): proxy"),
            "{text}"
        );
    }

    #[test]
    fn router_unknown_key_is_rejected() {
        assert!(message(with("[router]\nretries = 3\n")).contains("router: unknown key(s)"));
    }

    #[test]
    fn wrong_types_are_rejected_with_the_key_path() {
        for (user, path) in [
            ("[sessions]\nenabled = \"yes\"\n", "sessions.enabled"),
            ("[logging]\nbackups = 2.5\n", "logging.backups"),
            ("[providers.grok]\nmodel = 7\n", "providers.grok.model"),
            ("[providers.grok]\nmodel = \"  \"\n", "providers.grok.model"),
            (
                "[routing.code]\nbackup = \"openrouter\"\n",
                "routing.code.backup",
            ),
            ("[routing.code]\nbackup = [1]\n", "routing.code.backup"),
            ("providers = 5\n", "providers"),
        ] {
            let text = message(with(user));
            assert!(text.contains(path), "{user:?} -> {text}");
        }
    }

    #[test]
    fn numeric_limits_are_enforced() {
        for user in [
            "[logging]\nmax_bytes = 9999\n",
            "[logging]\nmax_bytes = 100000001\n",
            "[logging]\nbackups = 21\n",
            "[logging]\nbackups = -1\n",
            "[sessions]\nmax_entry_chars = 99\n",
            "[sessions]\nmax_entry_chars = 100001\n",
        ] {
            assert!(with(user).is_err(), "{user}");
        }
        assert!(with("[logging]\nmax_bytes = 10000\nbackups = 20\n").is_ok());
    }

    #[test]
    fn log_level_is_validated_and_normalized() {
        assert_eq!(
            with("[logging]\nlevel = \"debug\"\n")
                .unwrap()
                .logging()
                .level,
            LogLevel::Debug
        );
        assert!(message(with("[logging]\nlevel = \"loud\"\n")).contains("must be one of DEBUG"));
    }

    #[test]
    fn timeout_must_be_a_number_in_range() {
        for bad in ["0", "0.5", "301", "\"60\"", "true", "nan", "inf", "-5"] {
            let user = format!("[providers.grok]\ntimeout_s = {bad}\n");
            assert!(with(&user).is_err(), "{bad}");
        }
        for good in ["1", "60", "60.5", "300"] {
            let user = format!("[providers.grok]\ntimeout_s = {good}\n");
            assert!(with(&user).is_ok(), "{good}");
        }
    }

    #[test]
    fn router_total_timeout_is_validated() {
        assert!(with("[router]\ntotal_timeout_s = 4\n").is_err());
        assert!(with("[router]\ntotal_timeout_s = 901\n").is_err());
        assert!(with("[router]\ntotal_timeout_s = \"60\"\n").is_err());
        assert_eq!(
            with("[router]\ntotal_timeout_s = 30\n")
                .unwrap()
                .router()
                .total_timeout_s,
            30.0
        );
    }

    #[test]
    fn invalid_names_are_rejected() {
        for user in [
            "[providers.\"Bad Name\"]\nkind = \"google\"\n",
            "[providers.\"1st\"]\nkind = \"google\"\n",
            "[providers.grok]\nkeyring_name = \"Has Space\"\n",
            "[routing.\"UPPER\"]\nprimary = \"grok\"\n",
        ] {
            assert!(message(with(user)).contains("invalid name"), "{user}");
        }
        assert!(is_valid_name(&"a".repeat(32)) && !is_valid_name(&"a".repeat(33)));
    }

    #[test]
    fn route_must_reference_known_providers() {
        let text = message(with("[routing.code]\nprimary = \"nope\"\n"));
        assert!(
            text.contains("routing.code: unknown provider(s): nope"),
            "{text}"
        );
        assert!(with("[routing.code]\nbackup = [\"ghost\"]\n").is_err());
    }

    #[test]
    fn route_rejects_duplicate_providers() {
        let text = message(with(
            "[routing.code]\nprimary = \"deepseek\"\nbackup = [\"openrouter\", \"deepseek\"]\n",
        ));
        assert!(text.contains("more than once"), "{text}");
    }

    #[test]
    fn fallback_route_is_required_and_providers_cannot_be_empty() {
        let defaults: Table = DEFAULTS.parse().unwrap();
        let mut no_general = defaults.clone();
        if let Some(Value::Table(routing)) = no_general.get_mut("routing") {
            routing.remove(FALLBACK_ROUTE);
        }
        let text = parse_config(&no_general, None, None)
            .unwrap_err()
            .to_string();
        assert!(text.contains("the fallback route is required"), "{text}");

        let mut no_providers = defaults;
        no_providers.remove("providers");
        let text = parse_config(&no_providers, None, None)
            .unwrap_err()
            .to_string();
        assert!(text.contains("at least one provider"), "{text}");
    }

    // ----- secrets and endpoints -----

    #[test]
    fn secret_looking_keys_are_rejected_with_a_pointer_to_the_keyring() {
        for user in [
            "[providers.grok]\napi_key = \"x\"\n",
            "[providers.grok]\nAPI_KEY = \"x\"\n",
            "[providers.grok]\nauthorization = \"x\"\n",
            "[providers.grok]\ngrok_token = \"x\"\n",
            "[logging]\ndb_password = \"x\"\n",
            "token = \"x\"\n",
            "[providers.grok.headers]\nbearer = \"x\"\n",
        ] {
            let text = message(with(user));
            assert!(text.contains("kellu keys set"), "{user} -> {text}");
        }
    }

    #[test]
    fn legitimate_keys_with_similar_words_are_not_flagged() {
        for key in [
            "max_tokens",
            "keyring_name",
            "tokenizer",
            "secretary",
            "passwords_seen",
        ] {
            assert!(!is_secret_key(key), "{key}");
        }
        for key in [
            "api_key",
            "Token",
            "x_api_key",
            "db_secret",
            "ADMIN_PASSWORD",
        ] {
            assert!(is_secret_key(key), "{key}");
        }
        assert!(with("[providers.tokens_demo]\nkind = \"google\"\nbase_url = \"https://e.com\"\nmodel = \"m\"\ntimeout_s = 5\n").is_ok());
    }

    #[test]
    fn credentials_in_urls_are_rejected() {
        for url in [
            "https://user:pass@example.com/v1",
            "https://user@example.com",
        ] {
            let user = format!("[providers.grok]\nbase_url = \"{url}\"\n");
            let text = message(with(&user));
            assert!(
                text.contains("credentials in URLs are not allowed"),
                "{text}"
            );
            assert!(
                !text.contains("pass"),
                "the error must not echo the URL: {text}"
            );
        }
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        for url in [
            "ftp://example.com",
            "file:///etc/passwd",
            "example.com",
            "javascript:alert(1)",
        ] {
            let user = format!("[providers.grok]\nbase_url = \"{url}\"\n");
            assert!(with(&user).is_err(), "{url}");
        }
    }

    #[test]
    fn plain_http_is_only_allowed_for_localhost() {
        assert!(with("[providers.grok]\nbase_url = \"http://localhost:8080/v1\"\n").is_ok());
        assert!(with("[providers.grok]\nbase_url = \"http://127.0.0.1:8080/v1\"\n").is_ok());
        assert!(with("[providers.grok]\nbase_url = \"http://[::1]:8080/v1\"\n").is_ok());
        let text = message(with(
            "[providers.grok]\nbase_url = \"http://example.com/v1\"\n",
        ));
        assert!(
            text.contains("plain http is only allowed for localhost"),
            "{text}"
        );
    }

    #[test]
    fn errors_never_echo_configuration_values() {
        let secret = "sk-live-0123456789abcdef";
        for user in [
            format!("[providers.grok]\ntimeout_s = \"{secret}\"\n"),
            format!("[providers.grok]\nkind = \"{secret}\"\n"),
            format!("[providers.grok]\nbase_url = \"{secret}\"\n"),
            format!("[logging]\nlevel = \"{secret}\"\n"),
            format!("[sessions]\nenabled = \"{secret}\"\n"),
            format!("[routing.code]\nbackup = [\"{secret}\"]\n"),
            format!("[routing.code]\nprimary = \"{secret}\"\n"),
        ] {
            let text = message(with(&user));
            assert!(
                !text.contains(secret) || text.contains("unknown provider"),
                "{user} -> {text}"
            );
        }
    }

    #[test]
    fn keys_from_the_file_are_made_printable_and_bounded_in_errors() {
        let long = "x".repeat(200);
        let user = format!("\"{long}\\u001b[31m\" = 1\n");
        let text = message(with(&user));
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(text.len() < 200, "{text}");
    }

    // ----- loading from disk -----

    #[test]
    fn missing_user_file_means_defaults_only() {
        let dir = TempDir::new().unwrap();
        let config = load_config(Some(&dir.path().join("nope.toml"))).unwrap();
        assert_eq!(config.source(), None);
        assert_eq!(config.providers().len(), 5);
    }

    #[test]
    fn user_file_is_applied_and_reported_as_the_source() {
        let dir = TempDir::new().unwrap();
        let path = write_user_file(&dir, "[router]\ntotal_timeout_s = 30\n", 0o600);
        let config = load_config(Some(&path)).unwrap();
        assert_eq!(config.router().total_timeout_s, 30.0);
        assert_eq!(config.source(), Some(path.as_path()));
    }

    #[test]
    fn loading_creates_nothing_on_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub/config.toml");
        load_config(Some(&path)).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn invalid_toml_is_a_config_error_with_a_line_and_no_content() {
        let dir = TempDir::new().unwrap();
        let path = write_user_file(
            &dir,
            "[router]\ntotal_timeout_s = sk-live-0123456789\n",
            0o600,
        );
        let text = load_config(Some(&path)).unwrap_err().to_string();
        assert!(text.contains("invalid TOML at line 2"), "{text}");
        assert!(!text.contains("sk-live"), "{text}");
    }

    #[test]
    fn a_file_that_is_not_utf8_is_a_config_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let text = load_config(Some(&path)).unwrap_err().to_string();
        assert!(text.contains("not valid UTF-8"), "{text}");
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = TempDir::new().unwrap();
        let big = format!("# {}\n", "x".repeat(MAX_CONFIG_BYTES as usize));
        let path = write_user_file(&dir, &big, 0o600);
        let text = load_config(Some(&path)).unwrap_err().to_string();
        assert!(text.contains("larger than"), "{text}");
    }

    #[test]
    fn a_file_exactly_at_the_limit_is_accepted() {
        let dir = TempDir::new().unwrap();
        let mut text = String::from("# ");
        text.push_str(&"x".repeat(MAX_CONFIG_BYTES as usize - 3));
        text.push('\n');
        assert_eq!(text.len() as u64, MAX_CONFIG_BYTES);
        let path = write_user_file(&dir, &text, 0o600);
        assert!(load_config(Some(&path)).is_ok());
    }

    #[test]
    fn group_or_world_writable_file_is_rejected() {
        let dir = TempDir::new().unwrap();
        for mode in [0o664, 0o666, 0o620, 0o602] {
            let path = write_user_file(&dir, "", mode);
            let result = load_config(Some(&path));
            assert!(
                matches!(result, Err(KelluError::Security(ref m)) if m.contains("chmod go-w")),
                "mode {mode:o}"
            );
        }
        for mode in [0o600, 0o644, 0o640] {
            let path = write_user_file(&dir, "", mode);
            assert!(load_config(Some(&path)).is_ok(), "mode {mode:o}");
        }
    }

    #[test]
    fn symlinked_config_is_rejected() {
        let dir = TempDir::new().unwrap();
        let target = write_user_file(&dir, "", 0o600);
        let link = dir.path().join("link.toml");
        symlink(&target, &link).unwrap();
        assert!(matches!(
            load_config(Some(&link)),
            Err(KelluError::Security(_))
        ));
    }

    #[test]
    fn a_directory_or_fifo_in_place_of_the_file_is_rejected_without_hanging() {
        let dir = TempDir::new().unwrap();
        let as_dir = dir.path().join("dir.toml");
        fs::create_dir(&as_dir).unwrap();
        assert!(matches!(
            load_config(Some(&as_dir)),
            Err(KelluError::Security(_))
        ));

        let fifo = dir.path().join("fifo.toml");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        assert!(matches!(
            load_config(Some(&fifo)),
            Err(KelluError::Security(_))
        ));
    }

    #[test]
    fn a_file_owned_by_another_user_is_rejected() {
        if !rustix::process::getuid().is_root() {
            return; // chown needs root
        }
        let dir = TempDir::new().unwrap();
        let path = write_user_file(&dir, "", 0o600);
        std::os::unix::fs::chown(&path, Some(65534), None).unwrap();
        assert!(matches!(
            load_config(Some(&path)),
            Err(KelluError::Security(_))
        ));
    }

    #[test]
    fn a_secret_in_the_user_file_is_rejected_when_loading() {
        let dir = TempDir::new().unwrap();
        let path = write_user_file(&dir, "[providers.grok]\napi_key = \"sk-live-123\"\n", 0o600);
        let text = load_config(Some(&path)).unwrap_err().to_string();
        assert!(
            text.contains("kellu keys set") && !text.contains("sk-live-123"),
            "{text}"
        );
    }
}
