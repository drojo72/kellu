# Security Policy

Kellu is an AI assistant that can read your code, call external model providers and, in later versions, run tools on your machine. That makes security a design requirement, not a feature added afterwards. This document describes how to report problems, the threat model, the controls we build, and, honestly, what is not covered.

> **Status: pre-0.1.0.** The controls below are the **design** that 0.1.0 must meet. They are tracked as milestones (see [Security roadmap](#security-roadmap)). Until a control ships, treat it as not implemented.

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

- Use GitHub's private vulnerability reporting (*Security → Report a vulnerability*) on this repository, or
- email `<security-contact-address>`.

Include the affected version or commit, steps to reproduce, the impact you expect, and whether you want credit. This is a small, maintained-by-one-person project: we aim to acknowledge reports within 7 days and to agree on a disclosure timeline with you. These are goals, not guarantees.

## Supported versions

| Version | Supported |
|---|---|
| `main` (pre-0.1.0) | Best effort |
| 0.1.x and later | Latest minor release |

## Threat model

### Assets

- API keys and infrastructure secrets (cloud, DNS, databases, GitHub tokens).
- Source code and private data in your projects.
- Integrity of your machine and accounts.
- Your consent: no destructive action without your explicit approval.

### Trust boundaries

| Zone | Trust level |
|---|---|
| Kellu core (keys, network, policy) | Trusted |
| Sandboxed tool processes | Untrusted execution |
| Extensions (agents, MCP servers) | Untrusted execution and untrusted data |
| Model providers | External: they see what you send them |
| Repository, web and stored content | Untrusted **data**, never instructions |
| The user | Trusted principal |

### Threats and controls

| # | Threat | Controls |
|---|---|---|
| T1 | **Prompt injection** through files, web pages or stored snippets | Content is treated as data. Privilege separation: the model proposes, the policy engine decides. Risk-based confirmation. Sandboxed execution. No secrets in model context. |
| T2 | **Data egress**: secrets or private code sent to a provider | Deny globs (`.env`, keys, certificates), secret redaction (patterns plus entropy), reversible placeholders, per-project local-only mode, size limits on context. |
| T3 | **Secret theft by tools** (reading `~/.ssh`, keyring, environment) | Sandbox with no access to home secrets or the keyring bus, environment allowlist, secrets never in argv, env or logs. |
| T4 | **Destructive actions** (deleting a repository, force-push, mass edits) | Risk levels, typed confirmation for destructive tools, git and `gh` writes executed on the host by the core, never by a model-controlled process. |
| T5 | **Path traversal and symlink escapes** in file tools | Paths resolved with `realpath` against a scope root, symlinks not followed outside it, deny globs, checks made on opened file descriptors where possible. |
| T6 | **Malicious or compromised dependency** | Core on the standard library where practical, optional extras, pinned versions (hashes for releases), `pip-audit` in CI, minimal dependency changes reviewed. |
| T7 | **Malicious or malformed provider response** (forged tool calls) | Tool calls validated against schema and policy before any execution. Model output is never executed directly. |
| T8 | **Other local users or processes** | Secrets in the OS keyring. State directories created with mode `0700`, files `0600`. |
| T9 | **Secrets leaking through logs and session history** | Redaction before writing, truncated content, no raw provider payloads by default. |
| T10 | **Untrusted extensions** (agents and MCP servers: tool poisoning, malicious tool descriptions, exfiltration) | Run as sandboxed processes with a scrubbed environment. Explicit per-server allowlist. Tool descriptions and outputs are untrusted data. Every tool has a declared risk level. No access to `provider`, `infra` or `db` secrets unless granted per server. Network only through the `network` profile, with confirmation. |
| T11 | **Configuration tampering** (redirecting provider endpoints to capture API keys, smuggling secrets into files) | TOML only, never code. Secret-looking keys are rejected. Endpoints must use HTTPS (plain HTTP only for localhost) and may not embed credentials. Unknown keys are errors. The user file must be a regular file, not a symlink, owned by the current user and not writable by group or others; it is opened without following symlinks and size-limited. |

### Out of scope

- A compromised operating system, kernel or root account, or physical access.
- A malicious user acting with their own privileges.
- Vulnerabilities inside the sandbox backend or the kernel itself (we configure bubblewrap correctly; we do not audit it).
- How a provider stores or uses the data you choose to send it. Choose providers accordingly.
- Cryptographic guarantees beyond the platform's. Kellu does not implement its own cryptography: transport security comes from the platform TLS stack, and secret storage from the OS keyring. Post-quantum protection therefore depends on those components and on your providers; we will track support rather than promise it.

## Security principles

1. **Fail closed.** If a protection cannot run, the capability it protects is disabled.
2. **Least privilege.** Every component receives the minimum access it needs.
3. **Defense in depth.** No single control (including the sandbox) is trusted alone.
4. **Secure defaults.** The safe option is the default; relaxing it is explicit and per session.
5. **Explicit consent.** Writing, deleting and network use are confirmed according to risk.
6. **Transparency.** What leaves your machine, and what a tool is about to do, is visible before it happens.

## Secrets management

- Secrets live in the OS keyring (Secret Service on Linux). They are not stored in dotfiles or `.env` files.
- Entries use the schema `application=kellu`, `name=<service>.<key>`, `tier=<tier>`.

| Tier | Examples | Availability |
|---|---|---|
| `provider` | model provider API keys | Used by the core in the normal flow |
| `infra` | cloud, DNS, deployment tokens | Not reachable by the assistant unless explicitly enabled, with confirmation |
| `db` | database passwords and admin keys | Same as `infra` |

- Secrets are never placed in the model context, in command-line arguments (visible in `/proc`), in the environment of sandboxed processes, or in logs.
- `kellu keys migrate` copies secrets from a previous keyring schema. It never deletes the originals; removal is a separate, confirmed step. `--dry-run` shows names only, never values.
- Use **least-privilege, expiring tokens**. Prefer fine-grained GitHub tokens without `delete_repo`, and rotate keys that were ever pasted into a terminal or chat.

## Tool risk levels

| Level | Examples | Default policy |
|---|---|---|
| `read` | list, read, search, `git status`, `git log` | Automatic, inside the sandbox or path policy |
| `write` | edit a file, `git commit` | Confirmation (configurable per scope) |
| `network` | install packages, fetch URLs | Confirmation, and only with the `network` sandbox profile |
| `destructive` | delete files or branches, force-push, repository deletion | Typed confirmation; never automatic |

Every tool declares its level in its registration. A tool without a declared level cannot run.

## Sandbox

Process-spawning tools (shell, tests, formatters) run inside **bubblewrap** on Linux.

- **Defaults:** no network, read-only system, the project directory as the only writable path, ephemeral `/tmp`, scrubbed environment, `--die-with-parent`, new session.
- **Not exposed:** `~/.ssh`, `~/.gnupg`, the keyring bus, other home directories.
- **Profiles:** `read-only`, `workspace-write`, `network` (requires confirmation), and `unsafe` (explicit, per session, always logged).
- **Fail closed:** if no backend is available, the shell tool is disabled. `kellu doctor` reports why.
- **Project virtual environments** are mounted read-only by default, so a poisoned package cannot persist code that later runs outside the sandbox.

Limits you should know:

- A sandbox reduces damage; it is not a virtual machine. Kernel or backend vulnerabilities can defeat it.
- Some distributions restrict unprivileged user namespaces (for example Ubuntu 24.04). `kellu doctor` checks this.
- The `network` profile deliberately weakens isolation.
- macOS and Windows backends are later work; until then those platforms run with a reduced tool set.

## Data egress and sanitization

- **Outbound:** deny globs, then secret redaction (known token formats, high-entropy strings, private key blocks), with reversible placeholders so answers can be re-hydrated locally.
- **Inbound:** all external content is untrusted. Filtering cannot stop prompt injection by itself, which is why privilege separation, confirmation and the sandbox exist.
- Redaction is **best effort**. Patterns and entropy checks miss things. Do not rely on them as the only protection for highly sensitive material; use local-only mode or do not send it.
- The terminal REPL and the LSP server send code context to the provider you configure. Until the egress controls ship, do not enable Kellu on files that contain secrets.

## Supply chain and development practices

- Minimal dependencies; heavier ones are optional extras (`[tui]`, `[local]`).
- Pinned versions, `pip-audit` and tests in CI, hashes for release artifacts.
- Security-relevant modules (vault, redaction, permissions, sandbox) are written test-first, including negative tests (for example: the sandbox cannot read `~/.ssh`, cannot reach the network, cannot write outside the project).
- Legacy prototype code is ported module by module against a checklist: known bugs fixed, no side effects on import, injected configuration, tests. Secrets and `.env` files are never committed.

## Security roadmap

| Milestone | Security deliverable |
|---|---|
| M0 | This document, `.gitignore` for secrets, CI with `pip-audit` |
| M1 (done) | No side effects on import; private state directories (0700) and files (0600) opened without following symlinks; strict, secret-free TOML configuration; redacting, rotating logs; session logs that sanitize control characters and cannot forge structure |
| M2 | Provider timeouts and fallback; schema validation of provider responses; keys passed by header, never in URLs |
| M3 | Keyring vault with tiers, migration, scrubbed environment, deny globs, redaction v1, risk-based permissions |
| M4 | Sandbox backend and policies, `kellu doctor`, escape tests |
| M5 | Tool registry requiring a declared risk level; host-side git and `gh` writes with confirmation |
| M6 | LSP and REPL honoring deny globs and redaction |
| 0.2.0 | Agent loop gated by the policy engine; injection-oriented tests |
| 0.4.0 | Memory store with acceptance criteria, expiry and "from memory" labeling |
| 0.5.0 | MCP client with per-server allowlist, sandboxing and risk mapping; extension tool descriptions treated as untrusted |

## Hardening checklist for users

- Install `bubblewrap` and run `kellu doctor`.
- Keep infrastructure and database secrets out of the `provider` tier.
- Use fine-grained, short-lived tokens; rotate anything that was ever exposed.
- Prefer local-only mode for sensitive projects.
- Review confirmations; do not enable the `unsafe` profile as a habit.
- Keep Kellu, bubblewrap and your kernel up to date.
