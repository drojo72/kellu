<div align="center">
  <img src="/assets/branding/svg/kellu-logo-color-dark.svg" alt="Kellu Logo" width="350" />
  <p><em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em></p>
</div>

---

# Changelog

All notable changes to the **Kellu** project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] - 2026-10-20

### Added
- Complete architecture migration from the legacy Python prototype to **Ruby 3.2+ Core and Rust native extension layout**.
- Modular CLI framework powered by `Thor` with isolated subcommands (`bin/kellu`).
- Native Linux Secret Service wrapper in `lib/kellu/security/keyring.rb` using `Open3` and `secret-tool`.
- Robust block parser (`[/N]`) to handle keyring labels and entry index structures without string cropping bugs.
- Comprehensive brand design suite: vector logos, typography guide, and terminal mockups in `/assets` and `/docs/branding`.
- Project documentation layout: `ARCHITECTURE.md`, `SECURITY.md`, and session logs in `/docs/bitacora/`.

### Fixed
- Resolved Thor syntax warnings by configuring explicit descriptions and `self.exit_on_failure? = true`.
- Fixed file autoload errors for early placeholders in `lib/kellu/core/config.rb` and `lib/kellu/core/router.rb`.
- Fixed key corruption issue for `ai_gemma_direct_api_key` caused by legacy attribute mismatches.

### Changed
- Standardized secret deletion and retrieval symmetry using strict label and service attributes matching `kai` / `kellu`.