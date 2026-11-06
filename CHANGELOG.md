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
