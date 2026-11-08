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

# Security Policy

Kellu is an AI assistant that can read your code, call external model providers and, in later versions, run tools on your machine. That makes security a design requirement, not a feature added afterwards. This document describes how to report problems, the threat model, the controls we build, and, honestly, what is not covered.

> **Status: pre-0.1.0.** The controls below are the **design** that 0.1.0 must meet, tracked as milestones (see [Security roadmap](#security-roadmap)). Until a control ships, treat it as not implemented.
>
> **Implemented and tested so far (M0 to M5):** no side effects at startup or construction; state directories `0700` and files `0600`, opened without following symlinks and verified on the open descriptor (T8); strict, secret-free configuration (T11); log and session files that strip control and format characters, cannot forge structure and cannot be redirected through a symlink (T9, partly); the provider transport (redirects refused, keys only in headers and scrubbed from errors, size and duration limits, https enforced) and validation of provider responses and tool calls (T7, T11, partly); the keyring vault with tiers, secrets read only through stdin and never through arguments, scrubbed child environments (T3, T8); redaction v1 applied by default to session logs (T9, best effort); deny globs, project scope opened beneath a directory descriptor without following symlinks, grants, risk-based permissions and a hash-chained audit log, **as tested building blocks** (T2, T5, partly); the sandbox (M4) and the tool layer (M5): every tool declares a risk that is enforced before it runs, files change only through an approved proposal applied by the core, commands run in the sandbox without network, and git and `gh` run on the host with a hardened environment (T1, T3, T4, T5, partly).
>
> **Not wired up yet:** the tools are built and tested (M5) but nothing connects them to a model: the terminal REPL and the LSP server (M6) are their first callers, own the confirmation prompts and will start the application log. No command sends file context to a provider today (`kellu providers test` sends a fixed prompt), but when the front ends arrive the outbound path must apply deny globs and redaction first: redaction is **best effort** and uses a fixed `[REDACTED]` marker (reversible placeholders are planned). Do not point Kellu at files that contain secrets.

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
| T4 | **Destructive actions** (deleting a repository, force-push, mass edits) | Risk levels, typed confirmation for destructive tools, git and `gh` writes executed on the host by the core, never by a model-controlled process. Implemented in M5: every write, push and pull request asks first, a deletion needs its path typed, and no tool forces a push, amends, rewrites history or deletes a branch. |
| T5 | **Path traversal and symlink escapes** in file tools | Paths resolved with `realpath` against a scope root, symlinks not followed outside it, deny globs, checks made on opened file descriptors where possible. Implemented in M5: files are read through the grant and opened beneath the root with `openat2`; applying a proposal uses a directory descriptor opened the same way and refuses links, files of other users and files with several hard links. |
| T6 | **Malicious or compromised dependency** | Minimal, audited dependency set (pure Rust where possible, trimmed features), `Cargo.lock` committed and `--locked` builds, `cargo deny` (advisories, licenses, sources) and `cargo audit` in CI, every new crate justified and reviewed. |
| T7 | **Malicious or malformed provider response** (forged tool calls) | Tool calls validated against schema and policy before any execution. Model output is never executed directly. |
| T8 | **Other local users or processes** | Secrets in the OS keyring. State directories created with mode `0700`, files `0600`. |
| T9 | **Secrets leaking through logs and session history** | Redaction before writing, truncated content, no raw provider payloads by default. |
| T10 | **Untrusted extensions** (agents and MCP servers: tool poisoning, malicious tool descriptions, exfiltration) | Run as sandboxed processes with a scrubbed environment. Explicit per-server allowlist. Tool descriptions and outputs are untrusted data. Every tool has a declared risk level. No access to `provider`, `infra` or `db` secrets unless granted per server. Network only through the `network` profile, with confirmation. |
| T11 | **Configuration tampering** (redirecting provider endpoints to capture API keys, smuggling secrets into files) | TOML only, never code. Secret-looking keys are rejected. Endpoints must use HTTPS (plain HTTP only for localhost) and may not embed credentials. Unknown keys are errors. The user file must be a regular file, not a symlink, owned by the current user and not writable by group or others; it is opened without following symlinks and size-limited. Error messages name the key and never echo values. |
| T12 | **Untrusted editor-side input over LSP** (a root that is the home directory, repository-supplied configuration, open documents that contain secrets) | Roots equal to `$HOME`, an ancestor of it or `/` are rejected, and a project marker or explicit confirmation is required. Settings that arrive from `.kateproject` may only tighten policy, never relax deny globs, sandbox profile or endpoints. Deny globs apply to documents received over LSP, which are never forwarded to a provider. Edits are limited to a validated proposal, inside the scope, with document versions. |

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

*(Implemented in M3. Store, lookup and clear use `secret-tool`; listing asks the Secret Service over D-Bus through `busctl` and never requests a secret value. Verified against gnome-keyring; KWallet is not verified.)*

- Secrets live in the OS keyring (Secret Service on Linux). They are not stored in dotfiles or `.env` files.
- Entries have one attribute named `kellu` whose value is `<service>_<key>` (for example `kellu=ai_openrouter_api_key`) and the label `KELLU: <name>`. It is the same shape the earlier prototype used, with a different attribute name, so existing entries and shell habits keep working (`secret-tool lookup kellu ai_openrouter_api_key`).
- **Tiers are not stored in the keyring.** Kellu derives them from the entry name: prefix `ai_` is `provider`; `github_`, `cloudflare_`, `render_`, `desec_` and `duckdns_` are `infra`; `neon_`, `supabase_` and `mysql_` are `db`. A name that matches nothing defaults to `infra`, so unknown secrets fail closed. A per-user override in `config.toml` is planned and not implemented.
- A provider's `keyring_name` in the configuration maps to the entry `ai_<keyring_name>_api_key`, and may only point into the `ai_*` space (tier `provider`), so a tampered configuration cannot read an `infra` or `db` secret and send it to another endpoint.

| Tier | Examples | Availability |
|---|---|---|
| `provider` | model provider API keys | Used by the core in the normal flow |
| `infra` | cloud, DNS, deployment tokens | Not reachable by the assistant unless explicitly enabled, with confirmation |
| `db` | database passwords and admin keys | Same as `infra` |

- Secrets are never placed in the model context, in command-line arguments (visible in `/proc`), in the environment of sandboxed processes, or in logs. Secrets are read from stdin or without echo, never from arguments.
- `kellu keys cleanup-legacy` lists the entries left by the earlier prototype's attribute (`kai`); with `--apply` it deletes only those that already exist as `kellu` entries and keeps the rest. It shows names only, never values. Copying old entries to the new attribute is not implemented: do it by hand.
- Use **least-privilege, expiring tokens**. Prefer fine-grained GitHub tokens without `delete_repo`, and rotate keys that were ever pasted into a terminal or chat.

## Tool risk levels

| Level | Examples | Default policy |
|---|---|---|
| `read` | list, read, search, propose a change (a proposal changes nothing), run a command in the sandbox, `git status`, `git diff`, `git log` | Automatic, inside the sandbox or path policy |
| `write` | apply an approved proposal, `git commit` | Confirmation (configurable per scope) |
| `network` | `git push`, `gh pr create` (installing packages or fetching URLs from the sandbox is not offered yet) | Confirmation, and only when the grant allows network; without it the call is refused and the user is not even asked |
| `destructive` | delete a file (as a proposal; the path is typed); force-push, branch deletion and repository deletion have no tool | Typed confirmation; never automatic |

Every tool declares its level in its registration. A tool without a declared level cannot run: in code the level is a required argument of `ToolSpec::new`, a test pins the level of every shipped tool, and the registry enforces the requirement before the tool runs. A decision that cannot be written to the audit log stops the call.

## Authorization model

Full design: [docs/design/permissions-and-sandbox.md](docs/design/permissions-and-sandbox.md).

- **Reading** is granted per project, as read-only, when you open it. A scope root equal to your home directory, an ancestor of it or `/` is refused.
- **Writing is never a standing permission.** The model proposes changes as data (diffs with hashes); you choose **deny**, **partial** (selected hunks) or **full** (all valid items of that proposal); only the core applies them, after rechecking hashes and scope. If any check fails, nothing is applied.
- **How a proposal is applied (M5).** Edits are resolved to positions in the exact file the user saw (identified by its SHA-256) when they are proposed, so nothing is searched for again later. Applying re-reads every file through a directory descriptor, refuses anything that changed, keeps a private backup of the originals (mode `0600`, under the state directory), writes through a temporary file and an atomic rename in the same directory, and undoes the earlier items if a later one fails. It does not create directories, refuses a file that belongs to another user or has several hard links, does not carry over extended attributes or ACLs, and holds no lock against another process: a file edited in the instant between the last hash check and the rename of that same file can still be replaced (the backup keeps the original).
- **The real project is never writable from a sandbox.** Tools that need to write run on a staging copy and their result comes back as another proposal.
- One `Grant` object derives both the path policy and the sandbox mounts, and a test asserts they match.
- Every proposal, decision and application is recorded in an append-only audit log (mode 0600, hash-chained).

## Sandbox

Process-spawning tools (shell, tests, formatters) run inside **bubblewrap** on Linux. Built in M4 (crate `kellu-sandbox`) and covered by escape tests against a real bubblewrap, and used since M5 by the `run_command` tool.

- **A staging copy, never the real project.** Only the files the `Grant` lets the session read are copied (through the same `openat2` scope, so symlinks, special files, denied names such as `.env` and `id_rsa`, and anything outside the root stay out) into a private directory that is mounted at `/work`. The real project is not mounted, not even read-only. After a run, `Staging::changes()` reports what was modified, created, deleted or unsafe (symlinks, devices); nothing is written back by the sandbox.
- **Defaults:** no network, read-only system directories (`/usr` and what merged-`/usr` systems link to it, plus the linker cache), empty tmpfs for `/home` and `/tmp`, an environment built from nothing (`PATH`, `HOME`, `TMPDIR` plus allowlisted locale variables, never anything that looks like a secret), all capabilities dropped, `--die-with-parent`, new session, and a PID namespace. Commands are an `argv` list after `--`: no shell, and the command can never be read as a bubblewrap option.
- **Not exposed:** `~/.ssh`, `~/.gnupg`, the keyring bus, other home directories, `/etc` (except the linker cache), `/var`, `/run`, `/mnt`, `/media`. Escape tests assert this.
- **User namespaces are required, not optional.** `--unshare-user` is explicit, so a missing user namespace is an error instead of a silent fallback to running unconfined with a setuid bubblewrap; `--disable-userns` stops the sandboxed process from creating more.
- **Profiles:** `read-only` and `workspace-write` (both on the staging copy). Network is not a profile: it follows the `Grant` and requires confirmation. `run_command` never gets the network, even when the session allows it for `git push`. The `unsafe` profile is not implemented.
- **Resource limits** (CPU time, address space, processes, file size, open files), a wall-clock timeout that kills the whole process group, and an output cap. The limits are applied by `prlimit` **inside** the sandbox, around the command. They cannot be applied around bubblewrap itself: `RLIMIT_NPROC` counts every process of the user, so a limit of 256 stops bubblewrap from creating namespaces on a desktop session with more processes (measured). If `prlimit` is missing, the run is refused instead of going ahead unlimited.
- **Fail closed:** if bubblewrap cannot create the sandbox with these exact options, its version cannot be read, or limits are required but unavailable, nothing runs. `kellu doctor` reports why.

Limits you should know:

- A sandbox reduces damage; it is not a virtual machine. Kernel or backend vulnerabilities can defeat it. Memory safety in Rust is not isolation: the sandbox is.
- There is no seccomp filter yet: isolation comes from namespaces, dropped capabilities and resource limits. Landlock and a syscall filter are later work.
- Some distributions restrict unprivileged user namespaces (for example Ubuntu 24.04). `kellu doctor` checks this by building a real sandbox with Kellu's options and prints bubblewrap's own message when it fails.
- The `network` profile deliberately weakens isolation, and name resolution inside the sandbox is not verified (`/etc/resolv.conf` is often a link into `/run`, which is not mounted).
- Project virtual environments (`.venv`) are not mounted: symlinks are not copied, so a typical virtual environment does not work inside the sandbox yet.
- macOS and Windows backends are later work; until then those platforms run with a reduced tool set.
- **bubblewrap before 0.12.0 is affected by CVE-2026-87766** (GHSA-pxhw-h44j-8pfx: symlink traversal while bubblewrap creates files in attacker-controlled content during setup; fixed in 0.12.0). Kellu's mount layout never creates mount points inside a project tree (the staging directory is a single leaf mount at `/work` on a fresh tmpfs root, with no nested binds), so the precondition does not hold. By default an older bubblewrap is accepted and `kellu doctor` warns. Strict mode (`sandbox.require_patched_bwrap = true` in `config.toml`) refuses to run below 0.12.0. Distribution packages may carry the fix without a matching version number, or may drop it again: check your distribution's changelog.

## Git and `gh` on the host (M5)

Git needs the project's `.git` and the user's credentials, neither of which the sandbox has, so `git_*` and `gh_pr_create` run on the host. They are the one place where a program outside the sandbox acts on the project, so each call is narrow and bounded (120 s, 1 MiB of output, own process group).

- **Reading** (`git_status`, `git_diff`, `git_log`, automatic) runs with an environment built from nothing and with every configuration file outside the repository ignored. Command-line settings switch off the filesystem monitor, the pager, hooks and external transports, and diffs run with `--no-ext-diff --no-textconv --ignore-submodules=all`; paths are literal. Git does not know the deny list, so file names are listed first, denied ones are dropped, and only the rest is diffed.
- **A repository that names a program is refused.** Settings chosen by name inside the repository cannot be switched off from the command line. This was measured, not assumed: a `filter.<name>.clean` command in `.git/config` ran during `git status` and `git add` even with the hardening above. Every git call therefore first reads the repository's own configuration (which executes nothing) and refuses to run if it defines a filter driver, an include, `core.sshCommand`, `core.askPass` or `gpg.program`. A repository that arrives as an archive can carry its own `.git/config`; a repository you cloned cannot.
- **Writing** is three fixed command shapes with validated arguments, shown exactly before they run: `git_commit` (named paths only, message through stdin, hooks disabled), `git_push` (`HEAD` to a plain branch name on `origin`, never forced) and `gh_pr_create` (title and base are checked, the body goes through stdin). They use your own git configuration (name, e-mail, credentials), so the environment keeps only an allowlist (`HOME`, `XDG_CONFIG_HOME`, `SSH_AUTH_SOCK`, locale) and a fixed `PATH`: tokens in environment variables such as `GH_TOKEN` are **not** passed on. Push and pull request are `network`: refused unless the grant allows network, and then still confirmed.

Limits you should know: the list of dangerous configuration keys is a deny list, so a repository you do not trust should not be opened (a key that is not listed is not watched); commits do not run your `pre-commit` hooks; `git_diff` ignores submodules.

## Data egress and sanitization

- **Outbound:** deny globs, then secret redaction (known token formats, high-entropy strings, private key blocks), with reversible placeholders so answers can be re-hydrated locally (redaction v1 uses a fixed `[REDACTED]` marker; reversible placeholders are planned).
- **Inbound:** all external content is untrusted. Filtering cannot stop prompt injection by itself, which is why privilege separation, confirmation and the sandbox exist.
- Redaction is **best effort**. Patterns and entropy checks miss things. Do not rely on them as the only protection for highly sensitive material; use local-only mode or do not send it.
- The terminal REPL and the LSP server send code context to the provider you configure. Until the egress controls ship, do not enable Kellu on files that contain secrets.

## Supply chain and development practices

- Minimal dependencies with `default-features = false` where possible; heavier ones sit behind Cargo features. Every external crate is declared once in the workspace manifest with a one-line justification.
- `Cargo.lock` committed, `cargo deny` and `cargo audit` plus tests in CI, checksums and signatures for release artifacts. (The CI workflow exists; it has not run on GitHub yet, and `cargo audit` has not been run locally.)
- Memory safety and robustness: `unsafe_code = "forbid"` and denied `unwrap`, `expect` and `panic!` in non-test code, set once in the workspace lints and checked per crate by a test (a crash is a denial of service); secrets held in zeroizing types that never print (from M2).
- Security-relevant modules (vault, redaction, permissions, sandbox) are written test-first, including negative tests (for example: the sandbox cannot read `~/.ssh`, cannot reach the network, cannot write outside the project).
- The earlier prototypes (branches `original-python` and `kellu-prototype-ruby`) are behavioral references: the Python tests are the specification for the Rust port (see [docs/PORTING.md](docs/PORTING.md)). Secrets and `.env` files are never committed.

## Security roadmap

M0 to M2 were first implemented in the Python prototype and are being ported with behavior parity.

| Milestone | Status | Security deliverable |
|---|---|---|
| M0 | Done | This document, `.gitignore` for secrets, CI with `cargo deny` and `cargo audit`, private files and directories opened without following symlinks |
| M1 | Done | No side effects at startup or construction; strict, secret-free TOML configuration; rotating logs and session logs that sanitize control characters and cannot forge structure (the redaction hook is wired, the redactor arrives in M3) |
| M2 | Done | Endpoints must be https (http only for localhost) and are rechecked by the transport; redirects are refused so keys are never replayed to another host; response, line and stream-duration limits; keys travel only in headers, are validated against header injection and scrubbed from every error message; provider responses and tool calls are validated (names, JSON arguments, counts); a stream fails over only before any text was delivered; per-provider timeouts and a total time budget |
| M3 | Done | Keyring vault with tiers (a provider key name can only resolve into `ai_*`; `infra` and `db` have no read path), scrubbed environment, deny globs, redaction v1, risk-based permissions, project scope opened beneath a directory descriptor, `Grant`, hash-chained audit log, `kellu keys` and `kellu providers test`. Tested building blocks: nothing consumes grants, permissions, scope, deny globs or the audit log until M4 and M5 |
| M4 | Done | Sandbox backend (bubblewrap, `argv` without a shell), policy derived from the `Grant`, staging copy, resource limits inside the sandbox, `kellu doctor` (isolation, bubblewrap version, `prlimit`, `openat2`), escape tests. Tested building block: no tool runs in the sandbox until M5 |
| M5 | Done | Tool registry requiring a declared risk level and enforcing its requirement before a tool runs; proposal contract and host-side apply of approved hunks (hash recheck, private backup, atomic write, undo, typed deletion); sandboxed commands without network; hardened host-side git reads; git and `gh` writes with confirmation and no force. Tested building blocks: nothing calls the registry until M6 |
| M6 | Planned | LSP and REPL honoring deny globs and redaction |
| 0.2.0 | Planned | Agent loop gated by the policy engine; injection-oriented tests |
| 0.4.0 | Planned | Memory store with acceptance criteria, expiry and "from memory" labeling |
| 0.5.0 | Planned | MCP client with per-server allowlist, sandboxing and risk mapping; extension tool descriptions treated as untrusted |

## Hardening checklist for users

- Install `bubblewrap` and `util-linux` (for `prlimit`) and run `kellu doctor`.
- Keep infrastructure and database secrets out of the `provider` tier.
- Use fine-grained, short-lived tokens; rotate anything that was ever exposed.
- Prefer local-only mode for sensitive projects.
- Review confirmations; do not enable the `unsafe` profile as a habit.
- Keep Kellu, bubblewrap and your kernel up to date.
