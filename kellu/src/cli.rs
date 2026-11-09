//! Command-line parsing and dispatch.
//!
//! Exit codes: 0 ok, 1 error, 2 usage (clap).

use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand};
use kellu_common::app::{DISPLAY_NAME, NAME};
use kellu_core::config::{self, Config};
use kellu_providers::{KeyLookup, Provider, ProviderConfig, build_provider_capped};
use kellu_security::{SecretToolStore, Vault};
use secrecy::SecretString;

use kellu_frontends::theme::Theme;

use crate::{doctor, frontends_cmd, input, keys, providers_cmd, tools_cmd};

/// Success.
pub const EXIT_OK: u8 = 0;
/// A runtime error or a failed check.
pub const EXIT_ERROR: u8 = 1;

#[derive(Debug, Parser)]
#[command(
    name = NAME,
    version,
    about = format!("{DISPLAY_NAME}: a secure, modular AI assistant for the Kate editor.")
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check the environment (config, keyring and sandbox tooling)
    Doctor,
    /// Inspect the configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Manage API keys in the OS keyring (values are never printed)
    Keys {
        #[command(subcommand)]
        action: KeysAction,
    },
    /// Check the configured model providers
    Providers {
        #[command(subcommand)]
        action: ProvidersAction,
    },
    /// List the tools the assistant may use and what each needs from you
    Tools,
    /// Chat in the terminal (also what plain `kellu` does when run in a terminal)
    Chat,
    /// Run the language server for Kate on standard input and output (hover and completion)
    Lsp,
}

#[derive(Debug, Subcommand)]
enum KeysAction {
    /// List stored keys with their tier (never values)
    List,
    /// Store a key; the value is read from standard input (hidden on a terminal), never from
    /// arguments. NAME is a provider's short name (`openrouter`) or a full entry name.
    Set {
        /// Provider short name or full entry name
        name: String,
    },
    /// Remove a key
    Rm {
        /// Provider short name or full entry name
        name: String,
    },
    /// Find entries left by the earlier prototype; with --apply, delete those already migrated
    CleanupLegacy {
        /// Delete the entries that already exist as Kellu entries
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ProvidersAction {
    /// Send one tiny request to each provider (or just NAME) to check endpoint, model and key
    Test {
        /// Provider name from the configuration
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum ConfigAction {
    /// Print the user configuration file location
    Path,
    /// Validate the effective configuration
    Check,
    /// Show providers and routes (never secrets)
    Show,
}

pub(crate) fn say(out: &mut dyn Write, text: &str) {
    // A closed pipe (`kellu doctor | head`) must not crash the process.
    let _ = writeln!(out, "{text}");
}

fn load() -> kellu_common::Result<Config> {
    config::load_config(None)
}

/// Builds the vault when a command needs it (nothing touches the keyring before that).
pub type VaultFactory = Box<dyn Fn() -> kellu_common::Result<Vault>>;
/// Reads a secret from the user: prompt, then the stream for the prompt.
pub type SecretReader = Box<dyn Fn(&str, &mut dyn Write) -> kellu_common::Result<SecretString>>;
/// Builds a provider client from its configuration.
pub type ProviderBuilder = Box<dyn Fn(&ProviderConfig, &KeyLookup) -> Box<dyn Provider>>;

/// Probes the sandbox given the paths of `bwrap` and, if found, `prlimit`.
pub type ProbeBox = Box<dyn Fn(&Path, Option<&Path>) -> kellu_sandbox::Probe>;
/// Standard input for the interactive commands.
pub type InputFactory = Box<dyn Fn() -> Box<dyn BufRead>>;

/// Everything the commands reach outside the process through, so tests can replace it.
pub struct Services {
    pub vault: VaultFactory,
    pub read_secret: SecretReader,
    /// Builds the client `providers test` uses: same as `build_provider` but with a short total cap.
    pub build_probe: ProviderBuilder,
    pub load: Box<dyn Fn() -> kellu_common::Result<Config>>,
    /// Probes bubblewrap (and `prlimit`) for `kellu doctor`.
    pub probe: ProbeBox,
    /// Builds the clients the assistant talks through (the full per-provider timeout).
    pub build_assistant_provider: ProviderBuilder,
    /// Standard input for `kellu chat` and `kellu lsp`.
    pub stdin: InputFactory,
    /// The terminal theme for `kellu chat`.
    pub theme: Box<dyn Fn() -> Theme>,
    /// Whether standard input and output are both terminals: bare `kellu` chats only then.
    pub interactive: Box<dyn Fn() -> bool>,
    /// The user's home directory (a project root that contains it is refused).
    pub home: Box<dyn Fn() -> kellu_common::Result<std::path::PathBuf>>,
}

impl Services {
    /// The real keyring (`secret-tool` from `PATH`), standard input and network providers.
    pub fn real() -> Self {
        Self {
            vault: Box::new(|| {
                let program = doctor::find_in_path("secret-tool").ok_or_else(|| {
                    kellu_common::KelluError::config(
                        "secret-tool not found; install libsecret-tools (run `kellu doctor`)",
                    )
                })?;
                // `busctl` is only needed to list keys; without it everything else still works.
                let store = SecretToolStore::new(program);
                let store = match doctor::find_in_path("busctl") {
                    Some(busctl) => store.with_busctl(busctl),
                    None => store,
                };
                Ok(Vault::new(Arc::new(store)))
            }),
            read_secret: Box::new(input::read_secret),
            build_probe: Box::new(|config, lookup| {
                let cap = std::time::Duration::try_from_secs_f64(config.timeout_s)
                    .unwrap_or(std::time::Duration::from_secs(60));
                build_provider_capped(config, lookup, cap)
            }),
            load: Box::new(load),
            probe: Box::new(|bwrap, prlimit| {
                kellu_sandbox::probe(bwrap, prlimit, &kellu_sandbox::SystemLayout::host())
            }),
            build_assistant_provider: Box::new(|config, lookup| {
                kellu_providers::build_provider(config, lookup)
            }),
            stdin: Box::new(|| Box::new(std::io::stdin().lock())),
            theme: Box::new(|| {
                Theme::detect(
                    &|name| std::env::var(name).ok(),
                    std::io::stdout().is_terminal(),
                )
            }),
            interactive: Box::new(|| {
                std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
            }),
            home: Box::new(kellu_security::scope::home_dir),
        }
    }
}

fn config_path(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match config::user_config_path() {
        Ok(path) => {
            say(out, &path.display().to_string());
            if !path.exists() {
                say(err, "(file does not exist; packaged defaults are in use)");
            }
            EXIT_OK
        }
        Err(error) => {
            say(err, &format!("error: {error}"));
            EXIT_ERROR
        }
    }
}

fn config_check(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match load() {
        Ok(config) => {
            let source = config.source().map_or_else(
                || "packaged defaults only".to_owned(),
                |p| p.display().to_string(),
            );
            say(
                out,
                &format!(
                    "ok: {} providers, {} routes ({source})",
                    config.providers().len(),
                    config.routing().len()
                ),
            );
            EXIT_OK
        }
        Err(error) => {
            say(err, &format!("error: {error}"));
            EXIT_ERROR
        }
    }
}

fn config_show(out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let config = match load() {
        Ok(config) => config,
        Err(error) => {
            say(err, &format!("error: {error}"));
            return EXIT_ERROR;
        }
    };
    say(out, "providers:");
    for (name, provider) in config.providers() {
        let key = match &provider.keyring_name {
            Some(keyring) => match kellu_security::tier::provider_entry(keyring) {
                Ok(entry) => format!("keyring: {entry}"),
                Err(_) => "keyring: invalid name".to_owned(),
            },
            None => "no key".to_owned(),
        };
        say(
            out,
            &format!(
                "  {name:<13} {:<14} {}  {}  ({key})",
                provider.kind.as_str(),
                provider.model,
                provider.base_url
            ),
        );
    }
    say(out, "routes:");
    for (task, route) in config.routing() {
        let chain: Vec<_> = route.chain().collect();
        say(out, &format!("  {task:<13} {}", chain.join(" -> ")));
    }
    say(
        out,
        &format!(
            "router: total time budget {}s",
            config.router().total_timeout_s
        ),
    );
    EXIT_OK
}

/// Run the CLI with the real services and return the process exit code.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with(args, out, err, &Services::real())
}

/// Run the CLI over `services` and return the process exit code.
pub fn run_with<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write, services: &Services) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let text = error.render().to_string();
            if error.use_stderr() {
                let _ = write!(err, "{text}");
            } else {
                let _ = write!(out, "{text}");
            }
            return u8::try_from(error.exit_code()).unwrap_or(EXIT_ERROR);
        }
    };
    let command = match cli.command {
        Some(command) => command,
        None if (services.interactive)() => Command::Chat,
        None => {
            say(out, &Cli::command().render_help().to_string());
            return EXIT_OK;
        }
    };
    match command {
        Command::Doctor => doctor::run(
            &doctor::find_in_path,
            &*services.load,
            &*services.probe,
            out,
        ),
        Command::Keys { action } => {
            let vault = match (services.vault)() {
                Ok(vault) => vault,
                Err(error) => {
                    say(err, &format!("error: {error}"));
                    return EXIT_ERROR;
                }
            };
            match action {
                KeysAction::List => keys::list(&vault, out, err),
                KeysAction::Set { name } => keys::set(&vault, &name, services, out, err),
                KeysAction::Rm { name } => keys::remove(&vault, &name, out, err),
                KeysAction::CleanupLegacy { apply } => {
                    keys::cleanup_legacy(&vault, apply, out, err)
                }
            }
        }
        Command::Providers {
            action: ProvidersAction::Test { name },
        } => {
            let config = match (services.load)() {
                Ok(config) => config,
                Err(error) => {
                    say(err, &format!("error: {error}"));
                    return EXIT_ERROR;
                }
            };
            let vault = match (services.vault)() {
                Ok(vault) => vault,
                Err(error) => {
                    say(err, &format!("error: {error}"));
                    return EXIT_ERROR;
                }
            };
            providers_cmd::test(&config, name.as_deref(), &vault, services, out, err)
        }
        Command::Tools => tools_cmd::list(out, err),
        Command::Chat => frontends_cmd::chat(services, out, err),
        Command::Lsp => frontends_cmd::lsp(services, out, err),
        Command::Config { action } => match action {
            ConfigAction::Path => config_path(out, err),
            ConfigAction::Check => config_check(out, err),
            ConfigAction::Show => config_show(out, err),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use kellu_common::ProviderError;
    use kellu_providers::{ChatRequest, ChatResponse, EventStream, StreamEvent};
    use kellu_security::{MemoryStore, SecretStore};
    use secrecy::ExposeSecret;

    use super::*;

    /// `(keyring_name asked for, secret found)` per provider call.
    type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

    struct Fake {
        name: String,
        outcome: Result<(), ProviderError>,
        keyring_name: Option<String>,
        lookup: KeyLookup,
        seen: Seen,
    }

    impl Provider for Fake {
        fn name(&self) -> &str {
            &self.name
        }
        fn supports_tools(&self) -> bool {
            false
        }
        fn chat(&self, _: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            if let Some(key) = &self.keyring_name {
                let found = (self.lookup)(key).map(|s| s.expose_secret().to_owned());
                self.seen.lock().unwrap().push((key.clone(), found));
            }
            self.outcome.clone().map(|()| ChatResponse {
                text: "ok".to_owned(),
                provider: self.name.clone(),
                model: None,
                tool_calls: Vec::new(),
                finish_reason: None,
                usage: None,
            })
        }
        fn stream(&self, _: &ChatRequest) -> EventStream {
            match &self.outcome {
                Ok(()) => Box::new(
                    vec![
                        Ok(StreamEvent::Text("ok".to_owned())),
                        Ok(StreamEvent::Done(Box::new(ChatResponse {
                            text: "ok".to_owned(),
                            provider: self.name.clone(),
                            model: None,
                            tool_calls: Vec::new(),
                            finish_reason: None,
                            usage: None,
                        }))),
                    ]
                    .into_iter(),
                ),
                Err(error) => Box::new(std::iter::once(Err(error.clone()))),
            }
        }
    }

    struct Harness {
        store: Arc<MemoryStore>,
        seen: Seen,
        services: Services,
    }

    fn defaults() -> kellu_common::Result<Config> {
        config::load_config(Some(Path::new("/nonexistent/kellu-test/config.toml")))
    }

    fn fake_builder(
        failing: &'static [(&'static str, &'static str)],
        seen: Seen,
    ) -> ProviderBuilder {
        Box::new(move |config, lookup| {
            let outcome = failing
                .iter()
                .find(|(name, _)| *name == config.name)
                .map_or(Ok(()), |(_, reason)| {
                    Err(ProviderError::new(&config.name, *reason).with_status(401))
                });
            Box::new(Fake {
                name: config.name.clone(),
                outcome,
                keyring_name: config.keyring_name.clone(),
                lookup: Arc::clone(lookup),
                seen: Arc::clone(&seen),
            })
        })
    }

    fn harness(failing: &'static [(&'static str, &'static str)], typed: &'static str) -> Harness {
        let store = Arc::new(MemoryStore::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (vault_store, seen_for_builder) = (Arc::clone(&store), Arc::clone(&seen));
        let services = Services {
            vault: Box::new(move || Ok(Vault::new(Arc::clone(&vault_store) as Arc<_>))),
            read_secret: Box::new(move |_, _| Ok(SecretString::from(typed.to_owned()))),
            build_probe: fake_builder(failing, Arc::clone(&seen_for_builder)),
            load: Box::new(defaults),
            probe: Box::new(|_, _| kellu_sandbox::Probe {
                version: Some(kellu_sandbox::FIRST_PATCHED),
                isolation: Ok(()),
                prlimit: true,
                openat2: true,
            }),
            build_assistant_provider: fake_builder(failing, Arc::clone(&seen_for_builder)),
            stdin: Box::new(|| Box::new(std::io::Cursor::new(Vec::new()))),
            theme: Box::new(|| Theme::at(kellu_frontends::theme::Level::Ascii)),
            interactive: Box::new(|| false),
            home: Box::new(|| Ok(std::path::PathBuf::from("/nonexistent/kellu-home"))),
        };
        Harness {
            store,
            seen,
            services,
        }
    }

    fn run_args(h: &Harness, args: &[&str]) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let argv = std::iter::once("kellu").chain(args.iter().copied());
        let code = run_with(argv, &mut out, &mut err, &h.services);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn set_list_and_rm_use_the_injected_vault() {
        let h = harness(&[], "sk-or-test-0123456789abcdef");
        let (code, out, _) = run_args(&h, &["keys", "set", "openrouter"]);
        assert_eq!(
            (code, out.trim()),
            (0, "stored ai_openrouter_api_key (provider)")
        );
        let (_, listing, _) = run_args(&h, &["keys", "list"]);
        assert!(
            listing.contains("provider  ai_openrouter_api_key"),
            "{listing}"
        );
        assert!(!listing.contains("sk-or-test"));
        let (code, out, _) = run_args(&h, &["keys", "rm", "openrouter"]);
        assert_eq!((code, out.trim()), (0, "removed ai_openrouter_api_key"));
        assert!(run_args(&h, &["keys", "list"]).1.contains("no keys stored"));
    }

    #[test]
    fn infra_keys_are_listed_with_their_tier() {
        let h = harness(&[], "ghp_0123456789abcdefghij");
        run_args(&h, &["keys", "set", "github_token"]);
        let (_, listing, _) = run_args(&h, &["keys", "list"]);
        assert!(listing.contains("infra     github_token"), "{listing}");
    }

    #[test]
    fn cleanup_legacy_is_a_dry_run_unless_applied() {
        let h = harness(&[], "sk-or-test-0123456789abcdef");
        run_args(&h, &["keys", "set", "openrouter"]);
        h.store.add_legacy("ai_openrouter_api_key");
        h.store.add_legacy("github_token");
        let (_, dry, _) = run_args(&h, &["keys", "cleanup-legacy"]);
        assert!(dry.contains("would be removed with --apply"), "{dry}");
        assert!(dry.contains("github_token: kept"), "{dry}");
        let (_, done, _) = run_args(&h, &["keys", "cleanup-legacy", "--apply"]);
        assert!(done.contains("ai_openrouter_api_key: removed"), "{done}");
        assert!(done.contains("github_token: kept"), "{done}");
    }

    #[test]
    fn providers_test_reports_each_provider_and_the_exit_code() {
        let h = harness(&[("deepseek", "bad key")], "x");
        let (code, out, _) = run_args(&h, &["providers", "test"]);
        assert_eq!(code, 1);
        assert!(
            out.contains("[FAIL] deepseek: [deepseek] bad key (HTTP 401)"),
            "{out}"
        );
        assert!(out.contains("[  ok] "), "{out}");
        let (code, out, _) = run_args(&h, &["providers", "test", "grok"]);
        assert_eq!(code, 0, "{out}");
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn providers_test_feeds_providers_from_the_vault_only_within_ai() {
        let h = harness(&[], "sk-or-test-0123456789abcdef");
        run_args(&h, &["keys", "set", "openrouter"]);
        h.store
            .set(
                "github_token",
                &SecretString::from("ghp_0123456789abcdefghij".to_owned()),
            )
            .unwrap();
        run_args(&h, &["providers", "test", "openrouter"]);
        let seen = h.seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![(
                "openrouter".to_owned(),
                Some("sk-or-test-0123456789abcdef".to_owned())
            )]
        );
        assert!(
            h.store
                .requested()
                .iter()
                .all(|entry| entry.starts_with("ai_"))
        );
    }

    #[test]
    fn a_secret_inside_a_provider_error_is_redacted_in_the_report() {
        let h = harness(
            &[("deepseek", "rejected key sk-leaked-0123456789abcdefghij")],
            "x",
        );
        let (_, out, _) = run_args(&h, &["providers", "test", "deepseek"]);
        assert!(!out.contains("sk-leaked"), "{out}");
        assert!(out.contains("[REDACTED]"), "{out}");
    }

    #[test]
    fn an_unknown_provider_is_an_error() {
        let h = harness(&[], "x");
        let (code, _, err) = run_args(&h, &["providers", "test", "nope\u{1b}[31m"]);
        assert_eq!(code, 1);
        assert!(err.contains("no provider named 'nope[31m'"), "{err:?}");
    }

    #[test]
    fn a_failing_vault_is_reported_not_panicked() {
        let mut h = harness(&[], "x");
        h.services.vault =
            Box::new(|| Err(kellu_common::KelluError::config("secret-tool not found")));
        let (code, _, err) = run_args(&h, &["keys", "list"]);
        assert_eq!(code, 1);
        assert!(err.contains("secret-tool not found"));
    }
    fn feed(h: &mut Harness, input: &'static [u8]) {
        h.services.stdin = Box::new(move || Box::new(std::io::Cursor::new(input.to_vec())));
    }

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn chat_answers_through_the_configured_provider() {
        let mut h = harness(&[], "x");
        feed(&mut h, b"hello\n/exit\n");
        let (code, out, err) = run_args(&h, &["chat"]);
        assert_eq!(code, EXIT_OK, "{err}");
        assert!(out.contains("<*> Kellu.") && out.contains("ok"), "{out}");
    }

    #[test]
    fn chat_still_starts_without_a_keyring_and_says_so() {
        let mut h = harness(&[], "x");
        h.services.vault = Box::new(|| Err(kellu_common::KelluError::config("no secret-tool")));
        feed(&mut h, b"/exit\n");
        let (code, _, err) = run_args(&h, &["chat"]);
        assert_eq!(code, EXIT_OK);
        assert!(err.contains("keyring unavailable"), "{err}");
    }

    #[test]
    fn bare_kellu_chats_on_a_terminal_and_prints_help_otherwise() {
        let mut h = harness(&[], "x");
        feed(&mut h, b"/exit\n");
        assert!(
            run_args(&h, &[]).1.contains("Usage"),
            "not a terminal: help"
        );
        h.services.interactive = Box::new(|| true);
        let (code, out, _) = run_args(&h, &[]);
        assert_eq!(code, EXIT_OK);
        assert!(out.contains("Kellu. /help"), "{out}");
    }

    #[test]
    fn lsp_serves_the_protocol_on_stdout_only_and_says_when_features_are_off() {
        let mut h = harness(&[], "x");
        let mut wire = frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
        wire.extend(frame(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#));
        wire.extend(frame(r#"{"jsonrpc":"2.0","method":"exit"}"#));
        h.services.stdin = Box::new(move || Box::new(std::io::Cursor::new(wire.clone())));
        let (code, out, err) = run_args(&h, &["lsp"]);
        assert_eq!(code, EXIT_OK, "{err}");
        let mut cursor = std::io::Cursor::new(out.into_bytes());
        let first = kellu_frontends::lsp::transport::read_message(&mut cursor)
            .unwrap()
            .unwrap();
        assert!(
            first["result"]["capabilities"]
                .get("hoverProvider")
                .is_none()
        );
        let second = kellu_frontends::lsp::transport::read_message(&mut cursor)
            .unwrap()
            .unwrap();
        assert_eq!(second["id"], 2);
        assert!(
            kellu_frontends::lsp::transport::read_message(&mut cursor)
                .unwrap()
                .is_none()
        );
        assert!(err.contains("hover and completion are off"), "{err}");
    }

    #[test]
    fn lsp_with_broken_framing_exits_with_an_error_and_a_clean_stdout() {
        let mut h = harness(&[], "x");
        feed(&mut h, b"this is not LSP\r\n\r\n");
        let (code, out, _) = run_args(&h, &["lsp"]);
        assert_eq!(code, EXIT_ERROR);
        assert!(out.is_empty(), "{out:?}");
    }
}
