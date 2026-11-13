<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
</p>
<p align="center">
    <em>Local-first, modular AI assistant & LSP CLI built with Rust</em>
</p>

---

# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- Packaged routes: `code` and `explain` now start with `gemma_direct` (the old primaries failed on
  balance or demand and made the first hover take seconds). Provider `groq` uses
  `llama-3.1-8b-instant`: `llama-3.3-70b-versatile` was retired and answered 404 (id not yet
  confirmed with a real call).

### Added
- Packaged provider `groq` (OpenAI-compatible, `llama-3.3-70b-versatile`); the `general` route is
  now `gemma_direct` -> `groq` -> `openrouter` (provisional, for testing).

### Fixed
- The redactor no longer hides long lowercase model ids such as `anthropic/claude-3.5-sonnet`.
- `kellu providers test` caps each probe at the provider's `timeout_s` and shows progress.
- The OpenRouter default model is `openrouter/free`.

### Changed
- The project is being rewritten in pure Rust. The Python prototype (milestones M0 to M2, 185 test
  cases) and the Ruby prototype are preserved on the `original-python` and `kellu-prototype-ruby`
  branches as behavioral references for the port.

### Added
- **M3** (new work, not part of the prototype port):
  - `kellu-security`: keyring vault with tiers deduced from the entry name (a provider's
    `keyring_name` can only resolve into `ai_*`; `infra` and `db` secrets can be stored, listed
    and removed but have no read path), scrubbed child environments (allowlist), redactor v1
    (token formats, private key blocks, assignments, known values, entropy), deny globs, project
    scope opened with `openat2` (`BENEATH`, `NO_SYMLINKS`) that refuses `/`, the home directory
    and its ancestors, `Grant` (read-only by default, per-proposal writes with the hash the user
    saw, network off, expiry), risk-based permissions, and a hash-chained append-only audit log.
  - CLI: `kellu keys list|set|rm|cleanup-legacy` (secrets only through stdin, hidden on a
    terminal) and `kellu providers test`. `kellu doctor` checks `busctl`.
  - Keyring access: `secret-tool` for store, lookup and clear; listing goes through the Secret
    Service over D-Bus with `busctl` and never requests a secret value. Verified against a real
    gnome-keyring (`kellu/tests/real-keyring.sh`).
  - `kellu-core`: sessions redact secrets by default (they used the identity function before).
  - Dependencies: `sha2` (and six RustCrypto crates below it), `zeroize` (already in the tree),
    `rustix` features `stdio` and `termios`.
- **M7** (LSP fix): when Kate sends the home directory as the project root (it does so when it
  finds no project), the server no longer disables itself for good: each document uses the
  project it lives in (the nearest directory with a marker such as `.git`, never the home directory
  or an ancestor). A valid root from the client is never widened. Before this, hover and completion
  returned nothing in that case and the only trace was a line in the private log.
- **M7** (LSP diagnostics): when hover or completion return nothing, the server now writes the
  reason once per cause to the private log (`lsp: hover: ...`): document refused (and why), empty
  model answer, or failed model request. Only paths and reasons are logged, never document text.
- **M7** (token budget): hover and completion request up to 1024 tokens (were 300 and 256). A
  model that reasons before answering (seen with the `openrouter/free` fallback) used the whole
  budget thinking and returned an empty answer with finish reason `length`. The answer is still cut
  to 4000 characters and the prompt still asks for a few sentences.
- **M7** (empty answers): when the model answers with nothing, the private log also names the
  provider, the finish reason and the tokens generated (never any text), to tell a model that spent
  its budget thinking from one that simply had nothing to say.
- **M7** (documentation): the Kate example in the READMEs now uses a key of its own
  (`kellu`, an absolute path and a list of highlighting modes: a key such as `python` replaces Kate's
  built-in server) and says what was verified (hover and completion on Rust files in a real Kate);
  HANDOFF records the design criteria and the plan for a Markdown grammar checker after 0.1.0.
- **M6** (new work):
  - `kellu-frontends` (new crate): the terminal REPL, the Kate LSP server and the terminal theme.
    `kellu chat` (also plain `kellu` when run in a terminal) streams answers; `kellu lsp` serves
    hover and completion on standard input and output. Hand-written JSON-RPC framing over
    `serde_json`: no `lsp-server` or `lsp-types` (no new external dependency).
  - `kellu-core::assist`: the single place where editor text becomes a model request: a bounded
    window around the cursor, secrets redacted, the text fenced as untrusted data (a hostile
    `</kellu-code>` is broken), control characters and fences removed from the answer, and the
    conversation of the REPL bounded and redacted on its way out.
  - `[lsp]` configuration: `hover` and `completion`, both **off by default** (an editor asks for
    them by itself, and each one sends code to a provider). A project (`.kateproject`) can switch a
    feature off through `initializationOptions`, never on.
  - LSP rules: only `file:` URIs; the document must be inside the project scope (refused: `/`, the
    home directory and its ancestors, a root without a project marker), pass the deny globs, and
    not be, or sit behind, a symlink; the check is repeated for every request. Hover answers are
    plain text; completion is one plain-text edit (never snippet syntax) and has no trigger
    characters. Full-text sync; documents over 2 MiB or past 64 are not kept; answers are cached
    per document version.
  - Terminal theme: `<*>` (ASCII), `‹ ◆ ›` (Unicode) or color; `NO_COLOR`, `TERM=dumb`, an unset
    `TERM`, a non-UTF-8 locale and output that is not a terminal lower the level.
  - `kellu chat` and `kellu lsp` start without a keyring (providers that need a key fail with a
    clear error). `kellu lsp` writes warnings to the private log, never to the protocol stream.
- **M5** (new work):
  - `kellu-tools` (new crate): a registry in which every tool declares its risk (a required
    argument of `ToolSpec::new`, pinned per tool by a test) and where the requirement for that risk
    is enforced before the tool runs; what a tool returns is redacted, stripped of control
    characters and bounded. Tools: `list_files`, `read_file`, `search_text`, `propose_edit`,
    `propose_create`, `propose_delete`, `run_command` (sandbox), `git_status`, `git_diff`,
    `git_log`, `git_commit`, `git_push`, `gh_pr_create`.
  - Proposals: edits are resolved to byte offsets against the SHA-256 of the file the user is
    shown, so approving some hunks is a plain selection. Only `apply` writes, through a directory
    descriptor opened beneath the project root: it rechecks every hash, keeps a private backup of
    the originals, writes through a temporary file and an atomic rename, and undoes what it did if
    a later step fails. Deleting needs the path typed.
  - `run_command` runs on the staging copy without network (even when the session allows network
    for `git push`); with `write_back` its changes return as a proposal.
  - Git and `gh` run on the host with a scrubbed environment; reads switch off every setting a
    repository could use to run a program, refuse a repository whose `.git/config` names a filter
    driver, and hide files the deny list hides. There is no force push, `--amend`, branch deletion
    or hook execution.
  - `kellu tools` lists the tools with their risk; `kellu doctor` also looks for `gh`.
  - `kellu-security`: `Scope::open_dir`, `Grant::without_network`. `kellu-sandbox`: `run_host`
    (the same timeout and output cap for the few commands that cannot run in the sandbox).
  - No new external dependencies; `Cargo.lock` only gains the crate.
- **M4** (new work):
  - `kellu-sandbox`: a policy derived from one `Grant` (readable files, network, profile `read-only`
    or `workspace-write`, limits, scrubbed environment), a staging copy of the granted files made
    through the `openat2` scope (private, size capped, deleted on drop) with `changes()` to report
    what a tool modified, created, deleted or left unsafe, the bubblewrap command line (an `argv`
    list, no shell, explicit `--unshare-user`, `--disable-userns`, all capabilities dropped, empty
    `/home` and `/tmp`), resource limits through `prlimit` inside the sandbox, a wall-clock timeout
    that kills the process group, an output cap and a probe of the host.
  - `kellu doctor` checks that bubblewrap can build the sandbox with Kellu's exact options, its
    version against 0.12.0 (a warning that names the advisory and the switch), that `prlimit` works
    inside the sandbox and that the kernel has `openat2`.
  - `config.toml`: `[sandbox] require_patched_bwrap` (default `false`).
  - `kellu-security`: `openat2_available()`.
  - No new external dependencies.
- **M2** (parity with the Python prototype; completes the port, 137 of 137 test functions mapped):
  - `kellu-providers`: provider-neutral types, the `Provider` trait, an OpenAI-compatible client
    (chat, streaming, tool calls), a Google client (Gemini and Gemma, streaming), a registry and a
    hardened HTTP transport on `ureq` with `rustls`.
  - `kellu-core`: the router (`classify_task` with accent folding, fallback along the configured
    chain, tool-support skipping, a total time budget) and its stream variant, which fails over
    only before any text was delivered. An HTTP 402 (no balance) falls back to the next provider.
  - `kellu-common`: `KelluError::AllProvidersFailed` and `ProviderFailure`.
  - Dependencies: `ureq`, `serde_json`, `secrecy` (22 more crates in the tree, including `ring`).
    `deny.toml` allows `CDLA-Permissive-2.0` for the bundled CA list.
- **M1** (parity with the Python prototype):
  - `kellu-core`: strict configuration over the packaged `defaults.toml` (secret-looking keys,
    plain http, URL credentials, unknown keys and unsafe files are rejected; errors name the key
    and never echo values), a private rotating log behind the `log` facade, and Markdown session
    logs.
  - `kellu-providers`: provider configuration types only (clients and the router arrive in M2).
  - `kellu-common`: text helpers (control and format character stripping, truncation, the
    redactor hook), endpoint rules, UTC timestamps and `read_trusted_file`.
  - `kellu config path|check|show`, and a configuration check in `kellu doctor`.
- **M0**: Cargo workspace (`kellu-common`, `kellu`), toolchain pinned to Rust 1.91.1 (edition 2024),
  `deny.toml`, CI workflow, the architecture test, `kellu --version`, `kellu doctor`, XDG paths,
  private files and directories, and the error types.
- `docs/PORTING.md`, `docs/porting/python-tests.txt` and `docs/porting/PARITY.md`: module mapping,
  invariants to preserve, proposed dependencies, the reference tests and their Rust counterparts.
- Design notes: permissions and sandbox, and what Kellu can do through Kate's LSP client.
- Cargo workspace architecture in `docs/ARCHITECTURE.md`.
- `HANDOFF.md`: state, decisions and open items of the port.

### Security
- Provider transport: redirects are refused and never followed; keys travel only in headers (never
  in URLs), are held in `secrecy` types, are validated against header injection and are scrubbed
  from every error and log message; endpoints are rechecked by the transport; response, SSE line
  and total-duration limits; every request is capped at 600 seconds (the prototype capped streams
  only). Plain-http endpoints never use a proxy from the environment (a proxy would see the key
  in cleartext); tool-call ids and provider labels are sanitized.
- Log records are written one per line: newlines in a message are escaped, so a message cannot
  forge further log entries (the Python prototype kept raw newlines).
- Files read as configuration are opened without blocking, so a FIFO placed at the path cannot hang
  the process.
- A session label is redacted before it is turned into a file name.
