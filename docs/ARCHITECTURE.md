<div align="center">
  <img src="/assets/branding/svg/kellu-logo-color-dark.svg" alt="Kellu Logo" width="350" />
  <p><em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em></p>
</div>

---

# Architecture & System Design

This document details the software architecture, design principles, layer hierarchy, and security boundaries for **Kellu**. 

---

## 1. Architectural Overview & Design Principles

Kellu is designed as a **local-first, modular AI assistant and Language Server Protocol (LSP) tool**. The architecture prioritizes system safety, local execution, explicit dependencies, and modular extensibility.

### Core Principles
- **Local-First Security**: Secrets and states reside exclusively within local operating system constructs (e.g., Linux Secret Service via `secret-tool`). No credentials are logged or written to plain text files.
- **Strict Layered Isolation**: Dependencies flow strictly downward in a Directed Acyclic Graph (DAG). High-level orchestrators may depend on lower-level utilities, but lower layers never reference upper layers.
- **Polyglot Hybrid Model**: High-level orchestration, CLI parsing, and workflow logic are handled in **Ruby** for expressiveness and rapid iteration. Performance-critical, low-latency, or heavy text parsing operations are delegated to compiled **Rust extensions** (`ext/`).
- **Sandboxed Execution**: External tools and agentic commands run under strict sandbox policies to prevent arbitrary side-effects on the host system.

---

## 2. Layered Architecture (Ruby Core)

The Ruby codebase under `lib/kellu/` is structured into well-defined functional layers. Higher layers can call into equal or lower layers, but lower layers must remain completely unaware of higher layers.

```text
+---------------------------------------------------------------+
|                      Frontends Layer                          |
|         (bin/kellu, CLI / Thor, LSP Server, REPL)             |
+---------------------------------------------------------------+
                               |
                               v
+---------------------------------------------------------------+
|                         Core Layer                            |
|        (Config Engine, System Prompts, Router, State)         |
+---------------------------------------------------------------+
               |                 |                 |
               v                 v                 v
+-----------------------+ +-------------+ +---------------------+
|    Projects Layer     | | Tools Layer | |   Providers Layer   |
| (Workspace, Indexing) | | (Executors) | | (DeepSeek, Open3) |
+-----------------------+ +-------------+ +---------------------+
               |                 |                 |
               +-----------------+-----------------+
                               |
                               v
+---------------------------------------------------------------+
|                       Sandbox Layer                           |
|               (Process Isolation, Policy Gate)                |
+---------------------------------------------------------------+
                               |
                               v
+---------------------------------------------------------------+
|                      Security Layer                           |
|        (Keyring Manager, Open3 Secret-Tool Wrapper)          |
+---------------------------------------------------------------+
                               |
                               v
+---------------------------------------------------------------+
|                       Common Layer                            |
|             (Custom Errors, Utilities, Logger)               |
+---------------------------------------------------------------+
```

### Layer Breakdown

1. **`Frontends` (`lib/kellu/cli.rb`, future `lib/kellu/lsp/`)**
   - **Role**: Entry points for human users or editor interfaces (VS Code, Neovim, Kate).
   - **Responsibilities**: Command parsing (Thor), output formatting, signals handling, and translation between external user requests and `Core` operations.

2. **`Core` (`lib/kellu/core/`)**
   - **Role**: Brain and orchestrator of the system.
   - **Responsibilities**: Application lifecycle, TOML configuration parsing (`config.rb`), request routing (`router.rb`), prompt template loading, and state orchestration.

3. **`Projects` (`lib/kellu/projects/`)**
   - **Role**: Context and workspace manager.
   - **Responsibilities**: Codebase structural analysis, local file indexing, context retrieval, and repository-level awareness.

4. **`Tools` (`lib/kellu/tools/`)**
   - **Role**: Agentic tool definitions and capability implementations.
   - **Responsibilities**: Execution routines for code execution, file modification, terminal commands, and API inspection.

5. **`Providers` (`lib/kellu/providers/`)**
   - **Role**: AI provider adapters.
   - **Responsibilities**: Standardizing external vendor APIs (DeepSeek, OpenRouter, Google AI, Local LLMs) into a unified internal message format.

6. **`Sandbox` (`lib/kellu/sandbox/`)**
   - **Role**: Safe execution context.
   - **Responsibilities**: Enforcing execution boundaries, environment restrictions, and policy checks before `Tools` or commands execute system-level processes.

7. **`Security` (`lib/kellu/security/`)**
   - **Role**: Secret management wrapper (`keyring_manager.rb`).
   - **Responsibilities**: Communicating safely with `secret-tool` via `Open3`, isolating tokens in-memory, and guarding against leaking API keys into logs or stack traces.

8. **`Common` (`lib/kellu/error.rb`, `lib/kellu/version.rb`)**
   - **Role**: Shared fundamental primitives.
   - **Responsibilities**: Base custom exception hierarchy (`Kellu::Error`), logging abstractions, and global constants.

---

## 3. Dependency & Import Rules

To prevent coupling and cyclic dependencies, Kellu strictly enforces the following dependency rules:

1. **Downward Flow Only**: Module imports (`require_relative`) must only point to equal or lower-level components.
2. **No Upward References**: Lower layers (e.g., `Security`, `Common`) must **never** reference `Core`, `CLI`, or `Providers`.
3. **Provider Isolation**: Adapters inside `lib/kellu/providers/` must be decoupled from each other. They interact solely via standardized interfaces provided by `Core` or `Common`.
4. **Error Bubble Up**: Exceptions generated in lower layers must inherit from `Kellu::Error` (defined in `Common`) and bubble upward cleanly to be caught by `Frontends` / `CLI`.

---

## 4. Native Rust Extensions (`ext/`)

For operations where Ruby's execution speed or GIL limits system throughput, native Rust extensions are introduced under `ext/kellu_native/`.

### Use Cases for Rust Extensions
- **High-Speed Tokenization & Parsing**: Processing large dynamic context windows and codebases for the LSP server.
- **Diff & Vector Computation**: Fast local similarity search and structural diffing.
- **Protocol Serialization**: Fast JSON-RPC parsing for high-throughput LSP communication.

### FFI / C-Extension Boundary
- **Magnus / rb-sys**: Used as the bridge between Ruby C-API and Rust.
- **Safety Guarantee**: Memory allocations within Rust extensions remain isolated; invalid Rust memory operations panic safely at the extension boundary rather than causing unhandled segfaults in the Ruby process.

---

## 5. Trust Boundaries & Security Model

```text
[ UNTRUSTED ZONE ] ──────────> [ CONTROLLED ZONE ] ──────────> [ TRUSTED LOCAL ZONE ]
 External AI API           Kellu Provider Adapters         Linux Secret Service
 (DeepSeek, OpenRouter)    & Input Sanitizers              (secret-tool / Keyring)
         │                         │                                 │
         └───────── Strict ────────┴────────── In-Memory ────────────┘
                    TLS Encryption             Open3 Stream
```

1. **System Credentials Boundary**:
   - Secrets are fetched on demand from `secret-tool` into Ruby memory.
   - Keys are never persisted to disk (`.env`, config TOML, or temporary files are strictly forbidden from storing plain text keys).
2. **External Data Boundary**:
   - Responses received from external LLMs are treated as **untrusted data**.
   - Model-generated commands must pass through the `Sandbox` policy engine before execution.
3. **Execution Boundary**:
   - Tool execution is explicit. Destructive commands require user authorization or strict sandboxing.

---

## 6. Future Component Integration Roadmap

- **LSP Server Component (`lib/kellu/lsp/`)**: Will attach to the `Frontends` layer, exposing Kellu capabilities directly to IDEs via stdio/sockets using JSON-RPC.
- **Local SQLite / Vector Database (`lib/kellu/database/`)**: Will integrate between `Projects` and `Core` to cache local codebase embeddings and conversation histories securely.
- **Rust Text Engine Integration**: Incremental substitution of bottleneck Ruby parsers with `ext/kellu_native` methods without changing public Ruby contracts.