//! Where secrets are stored: the OS keyring (Secret Service) through `secret-tool` and `busctl`.
//!
//! No Secret Service crate: the ones measured in M3 pull about 200 packages (D-Bus, an async
//! runtime, ICU) against about 60 for all of Kellu, and `secret-tool` keeps compatibility with entries
//! made by hand or by the earlier prototypes.
//!
//! * **Read, write, delete** go through `secret-tool` (`lookup`, `store`, `clear`).
//! * **Listing** cannot: `secret-tool search` needs an attribute/value pair, returned a single
//!   entry for `xdg:schema` against a real gnome-keyring, and prints secret values. So listing
//!   asks the Secret Service over D-Bus through `busctl` (systemd): `SearchItems` with an empty
//!   filter returns every item path, and the `Attributes` property of each item names the entry.
//!   **No secret value is ever requested when listing.**
//!
//! Rules this module keeps:
//! * secrets are written through **stdin**, never through `argv`;
//! * every child gets a scrubbed environment ([`crate::env::KEYRING_ALLOWLIST`]), a timeout and a
//!   bounded output;
//! * buffers that held output are wiped on drop;
//! * D-Bus object paths from the keyring are validated before they reach `argv`;
//! * error messages never contain secret values.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kellu_common::app::{KEYRING_ATTRIBUTE, LEGACY_KEYRING_ATTRIBUTE};
use kellu_common::text::{strip_control_chars, truncate};
use kellu_common::{KelluError, Result};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use crate::env::{KEYRING_ALLOWLIST, scrubbed_from_process};
use crate::tier::is_valid_entry_name;

/// Largest secret accepted or returned.
pub const MAX_SECRET_BYTES: usize = 8 * 1024;
const MAX_LIST_BYTES: usize = 4 * 1024 * 1024;
const MAX_ATTRIBUTES_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 4096;
/// Most keyring items a listing will look at.
const MAX_ITEMS: usize = 1000;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const ITEM_PATH_PREFIX: &str = "/org/freedesktop/secrets/collection/";

/// One keyring entry that Kellu knows about. The name is validated; the value is never loaded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryInfo {
    /// Entry name, for example `ai_openrouter_api_key`.
    pub name: String,
    /// Whether it carries the earlier prototype's attribute instead of Kellu's.
    pub legacy: bool,
}

/// A place that keeps named secrets.
pub trait SecretStore: Send + Sync {
    /// The secret of `entry`, or `None` when there is none.
    ///
    /// # Errors
    /// The store could not be reached or answered something unexpected.
    fn get(&self, entry: &str) -> Result<Option<SecretString>>;

    /// Store `secret` under `entry`, replacing any previous value.
    ///
    /// # Errors
    /// The store could not be reached or refused the write.
    fn set(&self, entry: &str, secret: &SecretString) -> Result<()>;

    /// Delete `entry` (a missing entry is not an error).
    ///
    /// # Errors
    /// The store could not be reached.
    fn remove(&self, entry: &str) -> Result<()>;

    /// Delete the entry `name` made by the earlier prototype (attribute `kai`).
    ///
    /// # Errors
    /// The store could not be reached.
    fn remove_legacy(&self, name: &str) -> Result<()>;

    /// Names of the entries Kellu made, without loading their values.
    ///
    /// # Errors
    /// The store could not be reached or answered something unexpected.
    fn list(&self) -> Result<Vec<EntryInfo>>;
}

// ------------------------------------------------------------------------ secret-tool ----

/// An external program and the label used in its error messages.
#[derive(Debug, Clone)]
struct Tool {
    label: &'static str,
    program: PathBuf,
    leading_args: Vec<String>,
}

impl Tool {
    fn new(label: &'static str, program: PathBuf) -> Self {
        Self {
            label,
            program,
            leading_args: Vec::new(),
        }
    }
}

/// The OS keyring, reached by running `secret-tool` (package `libsecret-tools`) and, for
/// listing only, `busctl` (systemd).
#[derive(Debug, Clone)]
pub struct SecretToolStore {
    secret_tool: Tool,
    busctl: Option<Tool>,
    env: Vec<(String, String)>,
    timeout: Duration,
}

struct Output {
    code: Option<i32>,
    stdout: Zeroizing<Vec<u8>>,
    stderr: String,
}

impl SecretToolStore {
    /// Use the `secret-tool` executable at `program` (resolve it with `PATH` beforehand).
    ///
    /// Nothing is run until a method is called. Without [`SecretToolStore::with_busctl`],
    /// listing fails with a clear message and everything else works.
    #[must_use]
    pub fn new(program: PathBuf) -> Self {
        Self {
            secret_tool: Tool::new("secret-tool", program),
            busctl: None,
            env: scrubbed_from_process(KEYRING_ALLOWLIST),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Use the `busctl` executable at `program` for listing.
    #[must_use]
    pub fn with_busctl(mut self, program: PathBuf) -> Self {
        self.busctl = Some(Tool::new("busctl", program));
        self
    }

    #[cfg(test)]
    fn for_test(dir: &std::path::Path, timeout: Duration) -> Self {
        let script = |name: &str, label: &'static str| Tool {
            label,
            program: PathBuf::from("/bin/sh"),
            leading_args: vec![dir.join(name).display().to_string()],
        };
        Self {
            secret_tool: script("fake.sh", "secret-tool"),
            busctl: Some(script("busctl.sh", "busctl")),
            env: Vec::new(),
            timeout,
        }
    }

    fn run(
        &self,
        tool: &Tool,
        args: &[&str],
        stdin: Option<&SecretString>,
        limit: usize,
    ) -> Result<Output> {
        let mut command = Command::new(&tool.program);
        command
            .args(&tool.leading_args)
            .args(args)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| KelluError::io(&tool.program, e))?;

        let stdout = child.stdout.take().map(|pipe| spawn_reader(pipe, limit));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| spawn_reader(pipe, MAX_STDERR_BYTES));
        if let (Some(secret), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // A child that exits early closes the pipe; its exit status tells the story.
            let _ = pipe.write_all(secret.expose_secret().as_bytes());
        }

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(KelluError::security(format!(
                        "{} did not answer in time (is the keyring locked?)",
                        tool.label
                    )));
                }
                Err(source) => {
                    let _ = child.kill();
                    return Err(KelluError::io(&tool.program, source));
                }
            }
        };

        let (stdout, overflow) = join_reader(stdout, tool.label)?;
        let (stderr, _) = join_reader(stderr, tool.label)?;
        if overflow {
            return Err(KelluError::security(format!(
                "{} produced more output than allowed",
                tool.label
            )));
        }
        let limit = NonZeroUsize::new(200).unwrap_or(NonZeroUsize::MIN);
        let stderr = truncate(
            &strip_control_chars(&String::from_utf8_lossy(&stderr)),
            limit,
        );
        Ok(Output {
            code: status.code(),
            stdout,
            stderr: stderr.trim().to_owned(),
        })
    }

    fn failure(tool: &Tool, action: &str, output: &Output) -> KelluError {
        let code = output
            .code
            .map_or_else(|| "signal".to_owned(), |c| c.to_string());
        let detail = if output.stderr.is_empty() {
            String::new()
        } else {
            format!(": {}", output.stderr)
        };
        KelluError::security(format!(
            "{} {action} failed (exit {code}){detail}",
            tool.label
        ))
    }

    fn clear(&self, attribute: &str, name: &str) -> Result<()> {
        let tool = &self.secret_tool;
        let output = self.run(tool, &["clear", attribute, name], None, MAX_STDERR_BYTES)?;
        match output.code {
            Some(0) => Ok(()),
            Some(1) if output.stderr.is_empty() => Ok(()),
            _ => Err(Self::failure(tool, "clear", &output)),
        }
    }
}

/// Read up to `limit` bytes into a buffer allocated once (no reallocation leaves copies behind).
fn spawn_reader<R: Read + Send + 'static>(
    mut pipe: R,
    limit: usize,
) -> thread::JoinHandle<(Zeroizing<Vec<u8>>, bool)> {
    thread::spawn(move || {
        let mut buffer = Zeroizing::new(Vec::with_capacity(limit + 1));
        let mut bounded = (&mut pipe).take(limit as u64 + 1);
        let _ = bounded.read_to_end(&mut buffer);
        let overflow = buffer.len() > limit;
        // Drain the rest so the child never blocks on a full pipe.
        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        (buffer, overflow)
    })
}

fn join_reader(
    handle: Option<thread::JoinHandle<(Zeroizing<Vec<u8>>, bool)>>,
    label: &str,
) -> Result<(Zeroizing<Vec<u8>>, bool)> {
    match handle {
        None => Ok((Zeroizing::new(Vec::new()), false)),
        Some(handle) => handle
            .join()
            .map_err(|_| KelluError::security(format!("{label} output could not be read"))),
    }
}

impl SecretStore for SecretToolStore {
    fn get(&self, entry: &str) -> Result<Option<SecretString>> {
        check_entry(entry)?;
        let tool = &self.secret_tool;
        let mut output = self.run(
            tool,
            &["lookup", KEYRING_ATTRIBUTE, entry],
            None,
            MAX_SECRET_BYTES,
        )?;
        match output.code {
            Some(0) if !output.stdout.is_empty() => {
                let bytes = std::mem::take(&mut *output.stdout);
                match String::from_utf8(bytes) {
                    Ok(text) => Ok(Some(SecretString::from(text))),
                    Err(error) => {
                        let mut bytes = error.into_bytes();
                        bytes.zeroize();
                        Err(KelluError::security("the stored secret is not valid UTF-8"))
                    }
                }
            }
            // `lookup` exits 1 with no output when nothing matches.
            Some(0 | 1) if output.stdout.is_empty() && output.stderr.is_empty() => Ok(None),
            _ => Err(Self::failure(tool, "lookup", &output)),
        }
    }

    fn set(&self, entry: &str, secret: &SecretString) -> Result<()> {
        check_entry(entry)?;
        if secret.expose_secret().len() > MAX_SECRET_BYTES {
            return Err(KelluError::config("the secret is too long"));
        }
        let tool = &self.secret_tool;
        let label = format!("KELLU: {entry}");
        let output = self.run(
            tool,
            &["store", "--label", &label, KEYRING_ATTRIBUTE, entry],
            Some(secret),
            MAX_SECRET_BYTES,
        )?;
        if output.code == Some(0) {
            Ok(())
        } else {
            Err(Self::failure(tool, "store", &output))
        }
    }

    fn remove(&self, entry: &str) -> Result<()> {
        check_entry(entry)?;
        self.clear(KEYRING_ATTRIBUTE, entry)
    }

    fn remove_legacy(&self, name: &str) -> Result<()> {
        check_entry(name)?;
        self.clear(LEGACY_KEYRING_ATTRIBUTE, name)
    }

    fn list(&self) -> Result<Vec<EntryInfo>> {
        let tool = self.busctl.as_ref().ok_or_else(|| {
            KelluError::config(
                "listing keys needs `busctl` (part of systemd) in PATH; \
                 keys can still be set, removed and used by name",
            )
        })?;
        // Two calls per item at most, each bounded by the timeout; this caps the whole listing.
        let deadline = Instant::now() + self.timeout * 2;
        let search = self.run(
            tool,
            &[
                "--user",
                "--json=short",
                "call",
                SERVICE,
                SERVICE_PATH,
                "org.freedesktop.Secret.Service",
                "SearchItems",
                "a{ss}",
                "0",
            ],
            None,
            MAX_LIST_BYTES,
        )?;
        if search.code != Some(0) {
            return Err(Self::failure(tool, "SearchItems", &search));
        }
        let mut entries = Vec::new();
        for path in parse_item_paths(&search.stdout)? {
            if Instant::now() >= deadline {
                return Err(KelluError::security("listing the keyring took too long"));
            }
            let attributes = self.run(
                tool,
                &[
                    "--user",
                    "--json=short",
                    "get-property",
                    SERVICE,
                    &path,
                    "org.freedesktop.Secret.Item",
                    "Attributes",
                ],
                None,
                MAX_ATTRIBUTES_BYTES,
            )?;
            // An item can vanish between the two calls; that is not an error.
            if attributes.code == Some(0) {
                entries.extend(parse_item_attributes(&attributes.stdout));
            }
        }
        entries.sort();
        entries.dedup();
        Ok(entries)
    }
}

fn check_entry(entry: &str) -> Result<()> {
    if is_valid_entry_name(entry) {
        Ok(())
    } else {
        Err(KelluError::config(
            "invalid key name (use a-z, 0-9, _ or -; at most 64 characters)",
        ))
    }
}

fn unexpected(what: &str) -> KelluError {
    KelluError::security(format!(
        "the keyring answered something unexpected ({what})"
    ))
}

/// Whether `path` is a plain D-Bus object path inside the collections tree. Anything else is
/// ignored, so a hostile keyring cannot put arguments or other objects on a `busctl` command line.
fn is_item_path(path: &str) -> bool {
    path.starts_with(ITEM_PATH_PREFIX)
        && path.len() <= 256
        && !path.contains("//")
        && !path.ends_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'/')
}

/// The item paths in a `SearchItems` answer (`{"type":"aoao","data":[[unlocked],[locked]]}`).
fn parse_item_paths(json: &[u8]) -> Result<Vec<String>> {
    let value: Value = serde_json::from_slice(json).map_err(|_| unexpected("not JSON"))?;
    if value.get("type").and_then(Value::as_str) != Some("aoao") {
        return Err(unexpected("wrong reply type"));
    }
    let groups = value
        .get("data")
        .and_then(Value::as_array)
        .filter(|groups| groups.len() == 2)
        .ok_or_else(|| unexpected("wrong reply shape"))?;
    let mut paths = Vec::new();
    for group in groups {
        let items = group
            .as_array()
            .ok_or_else(|| unexpected("wrong reply shape"))?;
        paths.extend(
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|path| is_item_path(path))
                .map(str::to_owned),
        );
    }
    if paths.len() > MAX_ITEMS {
        return Err(KelluError::security(format!(
            "the keyring holds more than {MAX_ITEMS} items; refusing to list them"
        )));
    }
    Ok(paths)
}

/// The Kellu entries named by one item's `Attributes`
/// (`{"type":"a{ss}","data":{"kellu":"ai_x_api_key","xdg:schema":"..."}}`). Items made by other
/// applications, and names that are not valid entry names, yield nothing.
fn parse_item_attributes(json: &[u8]) -> Vec<EntryInfo> {
    let Ok(value) = serde_json::from_slice::<Value>(json) else {
        return Vec::new();
    };
    if value.get("type").and_then(Value::as_str) != Some("a{ss}") {
        return Vec::new();
    }
    let Some(attributes) = value.get("data").and_then(Value::as_object) else {
        return Vec::new();
    };
    let name = |attribute: &str| {
        attributes
            .get(attribute)
            .and_then(Value::as_str)
            .filter(|name| is_valid_entry_name(name))
            .map(str::to_owned)
    };
    let mut found = Vec::new();
    if let Some(name) = name(KEYRING_ATTRIBUTE) {
        found.push(EntryInfo {
            name,
            legacy: false,
        });
    }
    if let Some(name) = name(LEGACY_KEYRING_ATTRIBUTE) {
        found.push(EntryInfo { name, legacy: true });
    }
    found
}

// ------------------------------------------------------------------------------ memory ----

/// An in-memory store for tests and for callers that must never touch the real keyring.
#[derive(Debug, Default)]
pub struct MemoryStore {
    secrets: Mutex<BTreeMap<String, String>>,
    legacy: Mutex<BTreeSet<String>>,
    requested: Mutex<Vec<String>>,
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend the earlier prototype stored an entry named `name`.
    pub fn add_legacy(&self, name: &str) {
        self.legacy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned());
    }

    /// Every entry name passed to `get` so far (to prove which entries were ever reached).
    #[must_use]
    pub fn requested(&self) -> Vec<String> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, entry: &str) -> Result<Option<SecretString>> {
        check_entry(entry)?;
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry.to_owned());
        let secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(secrets
            .get(entry)
            .map(|value| SecretString::from(value.clone())))
    }

    fn set(&self, entry: &str, secret: &SecretString) -> Result<()> {
        check_entry(entry)?;
        let mut secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        secrets.insert(entry.to_owned(), secret.expose_secret().to_owned());
        Ok(())
    }

    fn remove(&self, entry: &str) -> Result<()> {
        check_entry(entry)?;
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(entry);
        Ok(())
    }

    fn remove_legacy(&self, name: &str) -> Result<()> {
        check_entry(name)?;
        self.legacy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
        Ok(())
    }

    fn list(&self) -> Result<Vec<EntryInfo>> {
        let secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        let legacy = self.legacy.lock().unwrap_or_else(PoisonError::into_inner);
        let mut all: Vec<EntryInfo> = secrets
            .keys()
            .map(|name| EntryInfo {
                name: name.clone(),
                legacy: false,
            })
            .chain(legacy.iter().map(|name| EntryInfo {
                name: name.clone(),
                legacy: true,
            }))
            .collect();
        all.sort();
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    // Shapes captured from a real gnome-keyring through `busctl --json=short` (M3).
    const SEARCH_ITEMS: &str = r#"{"type":"aoao","data":[["/org/freedesktop/secrets/collection/login/1","/org/freedesktop/secrets/collection/login/3","/org/freedesktop/secrets/collection/login/2"],[]]}"#;
    const ATTRS_KELLU: &str = r#"{"type":"a{ss}","data":{"kellu":"ai_openrouter_api_key","xdg:schema":"org.freedesktop.Secret.Generic"}}"#;
    const ATTRS_LEGACY: &str = r#"{"type":"a{ss}","data":{"kai":"github_token","xdg:schema":"org.freedesktop.Secret.Generic"}}"#;
    const ATTRS_OTHER: &str = r#"{"type":"a{ss}","data":{"application":"firefox"}}"#;

    #[test]
    fn item_paths_come_out_of_a_real_search_answer() {
        let paths = parse_item_paths(SEARCH_ITEMS.as_bytes()).unwrap();
        assert_eq!(
            paths,
            [
                "/org/freedesktop/secrets/collection/login/1",
                "/org/freedesktop/secrets/collection/login/3",
                "/org/freedesktop/secrets/collection/login/2",
            ]
        );
    }

    #[test]
    fn locked_items_are_listed_too() {
        let json = r#"{"type":"aoao","data":[[],["/org/freedesktop/secrets/collection/login/9"]]}"#;
        assert_eq!(parse_item_paths(json.as_bytes()).unwrap().len(), 1);
    }

    #[test]
    fn an_unexpected_answer_is_an_error() {
        for bad in [
            "",
            "not json",
            r#"{"type":"as","data":["x"]}"#,
            r#"{"type":"aoao","data":[]}"#,
            r#"{"type":"aoao","data":[[]]}"#,
            r#"{"type":"aoao","data":[1,2]}"#,
            r#"{"data":[[],[]]}"#,
        ] {
            assert!(parse_item_paths(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hostile_paths_are_ignored() {
        let json = r#"{"type":"aoao","data":[["-x","/etc/passwd","/org/freedesktop/secrets/collection/login/1 --foo","/org/freedesktop/secrets/collection/../x","/org/freedesktop/secrets/collection//1","/org/freedesktop/secrets/collection/login/","/org/freedesktop/secrets/collection/lo\ngin/1","/org/freedesktop/secrets/session/1","/org/freedesktop/secrets/collection/login/7"],[]]}"#;
        assert_eq!(
            parse_item_paths(json.as_bytes()).unwrap(),
            ["/org/freedesktop/secrets/collection/login/7"]
        );
    }

    #[test]
    fn too_many_items_are_refused() {
        let items: Vec<String> = (0..=MAX_ITEMS)
            .map(|i| format!("\"/org/freedesktop/secrets/collection/login/{i}\""))
            .collect();
        let json = format!(r#"{{"type":"aoao","data":[[{}],[]]}}"#, items.join(","));
        assert!(parse_item_paths(json.as_bytes()).is_err());
    }

    #[test]
    fn attributes_name_kellu_and_legacy_entries() {
        assert_eq!(
            parse_item_attributes(ATTRS_KELLU.as_bytes()),
            [EntryInfo {
                name: "ai_openrouter_api_key".into(),
                legacy: false
            }]
        );
        assert_eq!(
            parse_item_attributes(ATTRS_LEGACY.as_bytes()),
            [EntryInfo {
                name: "github_token".into(),
                legacy: true
            }]
        );
    }

    #[test]
    fn items_of_other_applications_and_odd_values_yield_nothing() {
        for json in [
            ATTRS_OTHER,
            r#"{"type":"a{ss}","data":{"kellu":"Bad Name; rm -rf"}}"#,
            r#"{"type":"a{ss}","data":{"kellu":""}}"#,
            r#"{"type":"a{ss}","data":{"kellu":"../x"}}"#,
            r#"{"type":"a{ss}","data":{"kellu":5}}"#,
            r#"{"type":"as","data":["kellu"]}"#,
            r#"{"type":"a{ss}","data":[]}"#,
            "not json",
            "",
        ] {
            assert!(parse_item_attributes(json.as_bytes()).is_empty(), "{json}");
        }
    }

    // ---- fake `secret-tool` and `busctl`, run through /bin/sh so no new executable is made ----

    const CRUD: &str = r#"
printf '%s\n' "$*" >> "$D/argv"
case "$1" in
  lookup) [ -f "$D/value" ] && cat "$D/value" || exit 1 ;;
  store) cat > "$D/value" ;;
  clear) rm -f "$D/value" ;;
esac
"#;

    const BUSCTL: &str = r#"
printf '%s\n' "$*" >> "$D/busctl_argv"
case "$3" in
  call) cat "$D/items.json" 2>/dev/null || exit 1 ;;
  get-property) cat "$D/attrs-$(basename "$5")" 2>/dev/null || { echo 'Failed to get property' >&2; exit 1; } ;;
esac
"#;

    struct Fake {
        dir: tempfile::TempDir,
    }

    impl Fake {
        fn new(secret_tool: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let fake = Self { dir };
            fake.write_script("fake.sh", secret_tool);
            fake.write_script("busctl.sh", BUSCTL);
            fake
        }

        fn write_script(&self, name: &str, body: &str) {
            let script = format!(
                "PATH=/usr/bin:/bin\nD='{}'\n{body}\n",
                self.dir.path().display()
            );
            fs::write(self.dir.path().join(name), script).unwrap();
        }

        fn put(&self, name: &str, content: &str) {
            fs::write(self.dir.path().join(name), content).unwrap();
        }

        /// A keyring with the three items of the real capture.
        fn keyring(&self) -> &Self {
            self.put("items.json", SEARCH_ITEMS);
            self.put("attrs-1", ATTRS_KELLU);
            self.put("attrs-3", ATTRS_OTHER);
            self.put("attrs-2", ATTRS_LEGACY);
            self
        }

        fn store(&self, timeout: Duration) -> SecretToolStore {
            SecretToolStore::for_test(self.dir.path(), timeout)
        }

        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
        }
    }

    const QUICK: Duration = Duration::from_secs(10);

    #[test]
    fn listing_goes_through_busctl_and_finds_both_generations() {
        let fake = Fake::new(CRUD);
        let entries = fake.keyring().store(QUICK).list().unwrap();
        assert_eq!(
            entries,
            [
                EntryInfo {
                    name: "github_token".into(),
                    legacy: true
                },
                EntryInfo {
                    name: "ai_openrouter_api_key".into(),
                    legacy: false
                },
            ]
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn listing_never_runs_secret_tool_and_never_asks_for_a_secret() {
        let fake = Fake::new(CRUD);
        fake.keyring().store(QUICK).list().unwrap();
        assert_eq!(
            fake.read("argv"),
            "",
            "secret-tool must not be used to list"
        );
        let calls = fake.read("busctl_argv");
        assert!(calls.contains("SearchItems a{ss} 0"), "{calls}");
        assert!(
            calls.contains("org.freedesktop.Secret.Item Attributes"),
            "{calls}"
        );
        for forbidden in [
            "GetSecret",
            "GetSecrets",
            "Unlock",
            "Delete",
            "Secret.Session",
        ] {
            assert!(!calls.contains(forbidden), "{forbidden}: {calls}");
        }
        assert_eq!(
            calls.lines().count(),
            4,
            "one search plus one read per item"
        );
    }

    #[test]
    fn listing_an_empty_keyring_is_not_an_error() {
        let fake = Fake::new(CRUD);
        fake.put("items.json", r#"{"type":"aoao","data":[[],[]]}"#);
        assert!(fake.store(QUICK).list().unwrap().is_empty());
    }

    #[test]
    fn hostile_item_paths_never_reach_the_busctl_command_line() {
        let fake = Fake::new(CRUD);
        fake.put(
            "items.json",
            r#"{"type":"aoao","data":[["-x","/etc/passwd","/org/freedesktop/secrets/collection/login/1 --foo","/org/freedesktop/secrets/collection/login/1"],[]]}"#,
        );
        fake.put("attrs-1", ATTRS_KELLU);
        let entries = fake.store(QUICK).list().unwrap();
        assert_eq!(entries.len(), 1);
        let calls = fake.read("busctl_argv");
        assert!(
            !calls.contains("--foo") && !calls.contains("passwd"),
            "{calls}"
        );
    }

    #[test]
    fn an_item_that_vanishes_is_skipped() {
        let fake = Fake::new(CRUD);
        fake.keyring();
        fs::remove_file(fake.dir.path().join("attrs-1")).unwrap();
        let entries = fake.store(QUICK).list().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].legacy);
    }

    #[test]
    fn listing_without_busctl_explains_what_to_install() {
        let store = SecretToolStore::new(PathBuf::from("/usr/bin/secret-tool"));
        let error = store.list().unwrap_err().to_string();
        assert!(
            error.contains("busctl") && error.contains("systemd"),
            "{error}"
        );
    }

    #[test]
    fn a_busctl_failure_is_reported_without_control_characters() {
        let fake = Fake::new(CRUD);
        fake.write_script(
            "busctl.sh",
            "printf 'Failed\\033[31m to connect\\r\\n' >&2; exit 1",
        );
        let error = fake.store(QUICK).list().unwrap_err().to_string();
        assert!(
            error.contains("busctl SearchItems failed (exit 1)"),
            "{error}"
        );
        assert!(
            !error.contains('\u{1b}') && !error.contains('\r'),
            "{error:?}"
        );
    }

    #[test]
    fn a_hung_busctl_is_killed_after_the_timeout() {
        let fake = Fake::new(CRUD);
        fake.write_script("busctl.sh", "sleep 30");
        let started = Instant::now();
        let error = fake.store(Duration::from_millis(300)).list().unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(
            error.to_string().contains("busctl did not answer in time"),
            "{error}"
        );
    }

    #[test]
    fn a_garbage_search_answer_is_an_error() {
        let fake = Fake::new(CRUD);
        fake.put("items.json", "garbage");
        assert!(fake.store(QUICK).list().is_err());
    }

    #[test]
    fn set_get_remove_round_trip_through_the_tool() {
        let fake = Fake::new(CRUD);
        let store = fake.store(QUICK);
        assert!(store.get("ai_x_api_key").unwrap().is_none());
        store
            .set(
                "ai_x_api_key",
                &SecretString::from("sk-test-12345678".to_owned()),
            )
            .unwrap();
        let got = store.get("ai_x_api_key").unwrap().unwrap();
        assert_eq!(got.expose_secret(), "sk-test-12345678");
        store.remove("ai_x_api_key").unwrap();
        assert!(store.get("ai_x_api_key").unwrap().is_none());
    }

    #[test]
    fn the_secret_goes_through_stdin_and_never_through_argv() {
        let fake = Fake::new(CRUD);
        let store = fake.store(QUICK);
        store
            .set(
                "ai_x_api_key",
                &SecretString::from("sk-never-in-argv-123".to_owned()),
            )
            .unwrap();
        let argv = fake.read("argv");
        assert!(
            argv.contains("store --label KELLU: ai_x_api_key kellu ai_x_api_key"),
            "{argv}"
        );
        assert!(!argv.contains("sk-never-in-argv-123"));
        assert_eq!(fake.read("value"), "sk-never-in-argv-123");
    }

    #[test]
    fn a_hung_tool_is_killed_after_the_timeout() {
        let fake = Fake::new("sleep 30");
        let started = Instant::now();
        let error = fake
            .store(Duration::from_millis(300))
            .get("ai_x_api_key")
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(
            error.to_string().contains("did not answer in time"),
            "{error}"
        );
    }

    #[test]
    fn oversized_output_is_refused() {
        let fake = Fake::new("head -c 20000 /dev/zero | tr '\\0' 'a'");
        let error = fake.store(QUICK).get("ai_x_api_key").unwrap_err();
        assert!(
            error.to_string().contains("more output than allowed"),
            "{error}"
        );
    }

    #[test]
    fn failures_are_reported_without_control_characters_or_values() {
        let fake = Fake::new("printf 'locked\\033[31m collection\\r\\n' >&2; exit 2");
        let error = fake
            .store(QUICK)
            .get("ai_x_api_key")
            .unwrap_err()
            .to_string();
        assert!(error.contains("exit 2"), "{error}");
        assert!(error.contains("locked"), "{error}");
        assert!(
            !error.contains('\u{1b}') && !error.contains('\r'),
            "{error:?}"
        );
    }

    #[test]
    fn a_stored_secret_that_is_not_utf8_is_an_error_not_a_value() {
        let fake = Fake::new("printf '\\377\\376\\375'");
        let error = fake.store(QUICK).get("ai_x_api_key").unwrap_err();
        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn the_child_does_not_inherit_the_environment() {
        // HOME is set in the test process; the scrubbed child must not see it.
        let fake = Fake::new("printf '%s' \"${HOME:-unset}\"");
        let value = fake.store(QUICK).get("ai_x_api_key").unwrap().unwrap();
        assert_eq!(value.expose_secret(), "unset");
    }

    #[test]
    fn hostile_names_never_reach_argv() {
        let fake = Fake::new(CRUD);
        let store = fake.store(QUICK);
        for bad in ["", "x y", "x\nclear", "../etc", "ai_x;rm", "--all"] {
            assert!(store.get(bad).is_err(), "{bad:?}");
            assert!(store.remove(bad).is_err(), "{bad:?}");
        }
        assert!(!Path::new(&fake.dir.path().join("argv")).exists());
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let store = SecretToolStore::new(PathBuf::from("/nonexistent/secret-tool"));
        assert!(store.get("ai_x_api_key").is_err());
    }

    #[test]
    fn the_memory_store_behaves_like_the_trait_says() {
        let store = MemoryStore::new();
        store
            .set("ai_a_api_key", &SecretString::from("v".to_owned()))
            .unwrap();
        store.add_legacy("ai_a_api_key");
        assert_eq!(store.list().unwrap().len(), 2);
        store.remove_legacy("ai_a_api_key").unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.requested(), Vec::<String>::new());
        let _ = store.get("ai_a_api_key").unwrap();
        assert_eq!(store.requested(), vec!["ai_a_api_key".to_owned()]);
    }
}
