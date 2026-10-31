<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Kellu 🤖

**Kellu** is a local-first, modular AI assistant and Language Server Protocol (LSP) CLI built for developers who demand privacy, speed, and seamless local keyring integration.

Transitioned from an early Python prototype to a high-performance **Ruby-core / Rust-extension** architecture, Kellu orchestrates multi-provider AI interactions while keeping credentials strictly secured within your Linux system keyring.

---

## 📸 Key Features

- **Local-First Security**: Native integration with Linux Secret Service (`secret-tool`) adhering to `org.freedesktop.Secret.Generic`. No API keys are stored in plain text.
- **Robust CLI**: Modular command-line interface powered by Thor with subcommand isolation.
- **Provider Aggregation**: Unified router interface to query multiple AI backends (DeepSeek, OpenRouter, Google AI, Cloudflare R2, etc.).
- **Bundler-Isolated Setup**: Standalone gem environment running on local Ruby 3.2+ with vendored dependencies.
- **Native Performance**: High-performance core with planned Rust extensions (`ext/`) for heavy text processing.

---

## 🏗 System Architecture & File Tree

```text
kellu/
├── assets/
│   └── branding/              # SVG & PNG brand assets
├── bin/
│   └── kellu                   # Main CLI executable (Thor entrypoint)
├── docs/
│   ├── ARCHITECTURE.md         # Layer definitions and dependency rules
│   ├── bitacora/               # Session development logs and index
│   ├── branding/               # Brand guidelines, typography & mockups
│   └── design/                 # Sandbox and LSP capabilities design
├── ext/                        # Rust C-extensions for core performance
├── lib/
│   ├── kellu/
│   │   ├── core/               # Configuration engine and provider router
│   │   ├── database/           # Local state storage
│   │   ├── providers/          # AI provider adapters
│   │   ├── security/           # Keyring manager wrapper (secret-tool)
│   │   ├── cli.rb              # Thor CLI subcommand definitions
│   │   ├── version.rb          # Kellu gem version (v0.1.0)
│   │   └── error.rb            # Custom error hierarchy
│   ├── tools/                  # Sandboxed execution tools
│   └── kellu.rb                # Root module entrypoint
├── test/                       # Test suite
├── vendor/
│   └── bundle/                 # Local vendored Ruby gems
├── Gemfile                     # Project dependencies definition
└── README.md                   # Project documentation
```

---

## 🚀 Getting Started

### Prerequisites

- **Ruby**: 3.2.3 or higher
- **System Tooling**: `build-essential`, `ruby-dev`, `libsecret-1-0`, `secret-tool`

### Installation & Setup

1. **Clone the repository**:
   ```bash
   git clone https://github.com/your-username/kellu.git
   cd kellu
   ```

2. **Install dependencies locally**:
   ```bash
   bundle config set --local path 'vendor/bundle'
   bundle install
   ```

3. **Verify the executable**:
   ```bash
   bundle exec bin/kellu version
   ```

---

## 🔑 Managing Credentials

Kellu stores credentials safely in the system keyring under the `kai` / `kellu` attributes.

```bash
# List available subcommands
bundle exec bin/kellu secret help

# Save a key securely
bundle exec bin/kellu secret set ai_gemma_direct_api_key "your_secret_api_key"

# Read a stored credential
bundle exec bin/kellu secret get ai_gemma_direct_api_key

# Delete a credential from the keyring
bundle exec bin/kellu secret delete ai_gemma_direct_api_key
```

---

## 🗺 Project Roadmap

- [x] **Phase 1: Architecture & Keyring Foundation**
  - [x] Transition from legacy Python prototype to Ruby/Rust architecture.
  - [x] Thor CLI structure with clean exception handling.
  - [x] Native `KeyringManager` via `Open3` and `secret-tool`.
  - [x] Audit and migration of system credentials.

- [/] **Phase 2: Configuration & Provider Router (In Progress)**
  - [ ] TOML config management in `lib/kellu/core/config.rb`.
  - [ ] Multi-provider API router in `lib/kellu/core/router.rb`.
  - [ ] System prompt & template loader.

- [ ] **Phase 3: Native Rust Extensions & LSP Server**
  - [ ] Rust C-extension integration for heavy text processing.
  - [ ] LSP protocol implementation for editor connectivity (Kate / Neovim / VS Code).

---

## 📄 License

MIT License. See `LICENSE` for details.
