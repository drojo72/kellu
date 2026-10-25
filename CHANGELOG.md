# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- M0: repository skeleton, `src/` layout, packaging, XDG paths, CI, layer-dependency tests.
- `kellu --version` and a basic `kellu doctor`.

- M1: strict TOML configuration with packaged defaults (`kellu config path|check`), private
  rotating logs with redaction hook, append-only session logs, private-file helpers.
- `kellu doctor` now validates the configuration.

- Design notes: permissions and sandbox (`docs/design/permissions-and-sandbox.md`) and what Kellu can do through Kate's LSP client (`docs/design/lsp-capabilities.md`).

### Changed
- Layer `stack` renamed to `projects` (discovery, stack detection, scopes).
- Architecture rule: packages are created when their milestone starts.
