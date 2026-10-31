<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
</p>
<p align="center">
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Porting plan: Python prototype to Rust

Status: the project is being rewritten in Rust. The Python prototype (milestones M0 to M2, 185 test
cases) is the **behavioral reference**: its designs, defaults and tests are the specification.
Behavior parity comes first; new work (M3 onwards) starts only after parity.

**Progress: M0 and M1 are ported (80 of 137 test functions mapped); M2 is next.** The register is
[porting/PARITY.md](porting/PARITY.md).

## Branches

- `original-python`: frozen Python prototype (M0 to M2, 185 test cases). Read-only reference and
  the main behavioral specification.
- `kellu-prototype-ruby`: frozen Ruby prototype (Thor CLI, keyring manager, TOML configuration with
  explicit `keyring_service`, DeepSeek, OpenRouter, Google and Gemma adapters, development log,
  brand assets). Second reference, mainly for keyring handling and provider integration lessons
  (see `HANDOFF.md`, section 0.1).
- `main`: the Rust implementation, with a fresh history.

Brand assets and the development log can be brought over without history:
`git checkout kellu-prototype-ruby -- assets docs/branding docs/bitacora`. The header of every
Markdown file points to `/assets/branding/svg/kellu-logo-color-*.svg`, so that checkout is needed
for the logo to render.

## What carries over unchanged

Design documents (`docs/design/`), the threat model, the TOML configuration schema and the packaged
`defaults.toml` (the Rust crate embeds the same file with `include_str!`), the keyring schema
and tier mapping, provider and route defaults, the CLI surface and exit codes (0 ok, 1 error,
2 usage, 3 not implemented), error semantics (`retryable` flags), the session log format, the
XDG paths, and the brand notes.

## Module mapping

| Python | Rust (crate: module) | Notes |
|---|---|---|
| `__app__.py` | `kellu-common: app` | Constants (`NAME`, `ENV_PREFIX`, keyring attribute `kellu`, legacy `kai`). Version from `CARGO_PKG_VERSION`. **Done (M0).** |
| `common/errors.py` | `kellu-common: error` | `thiserror` enums. `ProviderError` keeps `provider`, `status`, `retryable`, `reason`. **Done (M0).** |
| `common/net.py` | `kellu-common: net` | `endpoint_problem`, using `http::Uri` (not the `url` crate, see the dependency notes). **Done (M1).** |
| `common/text.py` | `kellu-common: text` | `strip_control_chars` also removes format characters (bidi overrides, zero-width) through an explicit range table, since `char::is_control` alone is not enough. **Done (M1).** |
| `common/paths.py` | `kellu-common: paths` | XDG, `ensure_private_dir`, `open_private_append`, `read_trusted_file`; `rustix` (safe API) for `O_NOFOLLOW` and descriptor-level checks. **Done (M0, M1).** |
| (new) | `kellu-common: utc` | UTC timestamps without a dependency. **Done (M1).** |
| `core/config.py`, `defaults.toml` | `kellu-core: config` | `toml` read as a table and validated by hand, so errors name the key path and never echo a value; the secret-key rejection and the numeric limits are kept. **Done (M1).** |
| `core/log.py` | `kellu-core: log` | `log` facade with a private rotating writer; redaction and control-character stripping on the final string. **Done (M1).** |
| `core/session.py` | `kellu-core: session` | Same Markdown format and state machine; entry kinds are an enum. **Done (M1).** |
| `providers/types.py` | `kellu-providers: types` | Provider configuration types only so far. **M1; the rest in M2.** |
| `core/router.py` | `kellu-core: router` | `classify_task` needs accent folding (Spanish); decide between `unicode-normalization` and a small fold table. **M2.** |
| `providers/*` | `kellu-providers: types, http, openai_compat, google, registry` | `Provider` trait; streaming as an iterator. **M2.** |
| `cli.py`, `doctor.py` | `kellu` (binary crate) | `clap`; the Python version check disappears. `config path|check|show` and the configuration check in `doctor` done in M1. |
| `tests/conftest.py` scripted server | `kellu-providers/tests/support` | `tiny_http` or a small `TcpListener` server. **M2.** |
| `test_architecture.py` | workspace test | Reads `cargo metadata` and asserts the allowed internal dependency edges. **Done (M0).** |

## Behaviors that must be preserved

- **Files:** state directories 0700 and files 0600; loose permissions are tightened; symlinks,
  non-regular files and files owned by another user are refused; files are opened without
  following symlinks and verified on the open descriptor.
- **Configuration:** unknown keys are errors; secret-looking keys are rejected with a pointer to the
  keyring; http only for localhost; no credentials in URLs; the user file must be a regular file,
  not a symlink, owned by the user, not group or world writable, at most 256 KiB; the result is
  immutable; loading creates nothing on disk.
- **Logging:** redaction applies to the final formatted record (including errors); control
  characters are stripped; rotation keeps 0600; configuring twice does not duplicate output.
- **Sessions:** the pipeline order is redact, strip control characters, truncate; content is fenced
  with a backtick run longer than any inside it; file names come from a sanitized slug; the provider
  name is sanitized; same-second sessions get unique names; start, log and end follow a state
  machine.
- **Providers:** keys travel only in headers (`Authorization: Bearer`, or `x-goog-api-key`) and never
  in URLs; keys with whitespace or control characters are rejected and a trailing newline is
  trimmed; errors are scrubbed (key replaced by `***`, control characters removed, 300 characters
  at most); redirects are refused; bodies are capped at 8 MiB, SSE lines at 1 MiB, streams at 600
  seconds; status 408, 409, 425, 429 and 5xx are retryable, network errors and timeouts are
  retryable, TLS errors and malformed responses are not; an HTTP 200 carrying an `error` object is an
  error; tool names match `^[A-Za-z0-9_.-]{1,64}$`; tool arguments must be a JSON object; at most 32
  tool calls; content given as parts is joined; Gemini "thought" parts are skipped; a blocked prompt
  is not retryable; Gemma receives the system message folded into the first user turn; Google does
  not support tools yet.
- **Router:** the chain comes from the task type; an HTTP 402 (insufficient balance) also falls back; any provider error falls back to the next
  provider; providers without tool support are skipped when tools are requested; a total time budget
  stops new attempts; a stream fails over only before the first text; a stream that ends without a
  final event counts as a failure; `classify_task` handles Spanish and English, with a regression
  test for the word "como".

The full list of test functions is in `docs/porting/python-tests.txt` (137 functions, 185 cases).
Every one needs a Rust counterpart or a written reason it does not apply. Track this in
`docs/porting/PARITY.md`.

## Decided differences from the prototype

Recorded in detail in `PARITY.md`: timestamps in logs and sessions are **UTC** (the prototype used
local time); the configuration is validated from a plain TOML table rather than through `serde`
derive; URLs are parsed with `http::Uri`.

## What Rust improves

- Cargo enforces the layer direction (a dependency cycle does not compile) and a metadata test
  rejects upward edges.
- `secrecy` and `zeroize` keep keys out of `Debug` output and wipe them on drop (M2).
- Typed, hand-validated configuration replaces the hand-written unknown-key checks.
- `cap-std` or `rustix` `openat2` with `RESOLVE_BENEATH` give descriptor-based scope checks for M3.
- Small binary and low memory for the many LSP instances Kate starts; no interpreter to install.
- Enums replace stringly typed roles, kinds and entry types.
- `unsafe_code = "forbid"` in every Kellu crate; clippy denies `unwrap`, `expect` and `panic!` in
  non-test code (a crash is a denial of service).
- `proptest` for the grant-to-mounts equivalence and for path scoping.

## Costs and watch-outs

- Rewriting M0 to M2. Mitigation: the designs are done and the tests are the specification.
- Compile times and dependency footprint. Keep crates small, set `default-features = false`, and
  justify every new crate.
- Capabilities to verify in the milestone that needs them: `ureq` (zero redirects, timeouts,
  streaming reads, and whether it parses URLs like the `http` crate), `rustls` root certificates.

## Dependencies

Chosen so far (each is justified in one line in the workspace `Cargo.toml`):

| Purpose | Crate | Milestone | Notes |
|---|---|---|---|
| Errors | `thiserror` | M0 | No `anyhow`: the binary uses `KelluError`. |
| CLI | `clap` (derive, trimmed features) | M0 | |
| Files and syscalls | `rustix` | M0 | Safe API for `O_NOFOLLOW`, `getuid`, errno. |
| URLs | `http` | M1 | Replaces the proposed `url`: it parses like the HTTP client will, and avoids the ICU dependency tree. |
| Configuration | `toml` (parse only) | M1 | Read as a table and validated by hand; no `serde` derive in Kellu's own code (the `toml` crate's `serde` feature is needed to read into a table). |
| Logging | `log` plus an own writer | M1 | |
| Time | own `kellu-common::utc` | M1 | Replaces the proposed `time`: logs use UTC, so no time-zone database is needed. |
| Tests | `tempfile`, `serde_json` (architecture test only) | M0 | |

Still proposed (confirm and justify per milestone):

| Purpose | Crate | Milestone |
|---|---|---|
| Serialization of provider payloads | `serde`, `serde_json` | M2 |
| HTTP and TLS | `ureq` with `rustls` | M2 |
| Secrets | `secrecy`, `zeroize` | M2 |
| Unicode folding for the router | `unicode-normalization` or a small fold table | M2 |
| Capability-based file access | `cap-std` | M3 |
| Hashing | `sha2` | M3 |
| Diffs | `similar`, own or `diffy` applier | M5 |
| Gitignore | `ignore` | M3 or M5 |
| LSP | `lsp-server`, `lsp-types` | M6 |
| Sandbox fallback | `landlock` | later |
| Tests | `tiny_http`, `proptest`, `assert_cmd`, `predicates`, `insta` | per milestone |
| CI tooling | `cargo-deny`, `cargo-audit` | M0 |

Keyring access in M3: either keep calling `secret-tool` (no new dependency, compatible with the
existing entries) or use a Secret Service crate that supports arbitrary attributes (which would
also allow `kellu keys list`). Decide in M3 after testing against the real schema `kellu=<name>`.

## Workspace layout

```
kellu/
├── Cargo.toml  Cargo.lock  rust-toolchain.toml  deny.toml
├── crates/
│   ├── kellu-common/      errors, XDG paths, private files, text helpers, endpoint rules, UTC
│   ├── kellu-security/    vault, redaction, deny globs, permissions, scope, grants, audit
│   ├── kellu-sandbox/     backend trait, bubblewrap, policy, staging
│   ├── kellu-providers/   types, HTTP transport, OpenAI-compatible, Google, registry
│   ├── kellu-projects/    discovery, stack detection, scopes
│   ├── kellu-tools/       registry, proposal and apply, file, shell and git tools
│   ├── kellu-core/        config, router, log, session (later cascade, agent loop)
│   └── kellu-frontends/   REPL, LSP, terminal theme (later gateway, ACP, MCP server)
├── kellu/                 binary crate: CLI, doctor
├── docs/  packaging/  tests/
```

Crates are created **when their milestone starts**: M0 `kellu-common` and `kellu`; M1 `kellu-core`
and `kellu-providers` (types only); M2 fills `kellu-providers`; M3 `kellu-security`; M4
`kellu-sandbox`; M5 `kellu-tools` and `kellu-projects`; M6 `kellu-frontends`.

## Port checklist

- [x] **M0:** workspace, pinned toolchain, `deny.toml`, CI (`cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings`, `cargo test --locked`, `cargo deny check`, `cargo audit`),
  `kellu --version`, `kellu doctor`, XDG paths, private files and directories, errors, the
  architecture test, LICENSE, `.gitignore` (including `target/`). (`cargo audit` not yet run
  locally; the CI workflow has not run on GitHub yet.)
- [x] **M1:** configuration with the same `defaults.toml`, logging, sessions, text helpers,
  `kellu config path|check|show`.
- [ ] **M2:** providers, the router, the scripted-server tests, provider listing in `config show`.
- [ ] **Gate:** every entry in `docs/porting/python-tests.txt` is mapped in `PARITY.md`.

## Verification environment

Install and use Rust 1.91 from apt (verified available, with clippy and rustfmt, and with crates
fetched from crates.io):

```
apt-get install -y cargo-1.91 rustc-1.91 rust-1.91-clippy rustfmt-1.91
export PATH=/usr/lib/rust-1.91/bin:$PATH
```

The installation does not persist between sessions; repeat it at the start of each one. Bubblewrap
tests also need `apt-get install -y bubblewrap`.
