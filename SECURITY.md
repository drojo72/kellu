<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcet="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Security Policy & Architecture

## Security Principles

Kellu is designed around a zero-trust model for untrusted input, model responses, and third-party extensions. The core principles are:

1. **Zero Plaintext Credentials**: No API keys, passwords, or tokens are ever written to disk or embedded in plain text configuration files.
2. **Keyring Isolation**: Credentials reside exclusively within the Linux Secret Service backend, accessible via `secret-tool` under the `org.freedesktop.Secret.Generic` schema.
3. **Fail-Closed Execution**: Any permission failure, invalid scope attempt, or hash mismatch halts operations immediately.
4. **Sandboxed Operations**: File operations and process executions triggered by AI tools or extensions run in restricted environments.

---

## Credential Storage (`KeyringManager`)

- **Backend**: Linux XDG Secret Service DBus interface executed using Ruby's `Open3`.
- **Attributes**: Schema matches `org.freedesktop.Secret.Generic` with `kai` / `kellu` service identifiers.
- **Parsing**: Line-by-line block parsing (`[/N]`) guarantees accurate extraction by label without misinterpreting compound service names.

---

## Scope & Path Restrictions

- **Root Containment**: All file operations are bounded by the project root. Root directories like `/`, `$HOME`, or user ancestor paths are rejected.
- **Automatic Denylist**: Access is strictly forbidden for system and sensitive files, including:
  - Credentials and environment files: `.env*`, `*.pem`, `*.key`, `id_*`, `credentials*`, `*.token`.
  - Version control internals: `.git/`.
- **Resolution**: Path resolution uses directory descriptor operations (`O_NOFOLLOW`) to prevent symlink traversal attacks (e.g., CVE-2026-87766).

---

## Sandbox & Execution Safety

- **Bubblewrap Aislamiento**: Process-spawning tools run isolated using `bubblewrap` with `--unshare-all`, cleared environments, read-only system mounts, and temporary file systems for `/tmp` and `$HOME`.
- **Staging Copies**: Tools that request file writes perform actions on temporary staging copies. Changes are validated through explicit proposal diffs requiring human approval before applying to the repository.

---

## Reporting Vulnerabilities

If you discover a security vulnerability within Kellu, please report it via email or open a private security advisory on the repository. Do not open public issues for sensitive security bugs.
