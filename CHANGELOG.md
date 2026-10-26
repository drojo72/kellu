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
- M2: provider layer (OpenAI-compatible client with streaming and tool calls, Google client for
  Gemini and Gemma, hardened HTTP transport) and the router with fallback chains, per-task
  classification and a total time budget. `kellu config show`.
- Default providers: OpenRouter, DeepSeek, Google (`gemini-flash-latest`), `gemma_direct`
  (Gemma 4 with its own key) and xAI Grok. Google endpoint moved to `v1beta`.

- Design notes: permissions and sandbox (`docs/design/permissions-and-sandbox.md`) and what Kellu can do through Kate's LSP client (`docs/design/lsp-capabilities.md`).

### Changed
- Layer `stack` renamed to `projects` (discovery, stack detection, scopes).
- Architecture rule: packages are created when their milestone starts.
