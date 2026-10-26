# Architecture

Kellu is a small headless core with replaceable adapters. This document defines the layers
and the dependency rule that tests enforce (`tests/unit/test_architecture.py`).

## Layers (0.1.0)

| Layer | Responsibility | May import |
|---|---|---|
| `common` | Errors, XDG paths, tiny shared utilities. Standard library only. | nothing |
| `security` | Vault, redaction, deny globs, permissions and policy. | `common` |
| `sandbox` | Sandbox interface, bubblewrap backend, profiles. | `common` |
| `providers` | Model provider clients (`OpenAICompatProvider`, `GoogleProvider`), HTTP transport, provider-neutral types. They receive their configuration and a key lookup; they never read either themselves. | `common` |
| `projects` | Project discovery, stack detection, scopes. | `common` |
| `tools` | Tool registry with schemas and mandatory risk levels; file, shell, git tools. | `common`, `security`, `sandbox` |
| `core` | Config, router (task classification, fallback chains, time budget), logging, sessions, (later) cascade and agent loop. | all layers above |
| `frontends` | Terminal REPL, Kate LSP, (later) gateway, ACP, MCP server. | all layers above and `core` |
| `cli.py`, `doctor.py` | Entry points. | everything |

```
frontends -> core -> tools -> sandbox, security -> common
                \-> providers -> common
                \-> projects  -> common
```

Rules:

1. Dependencies only point downward. A lower layer never imports a higher one.
2. Imports inside the package are **absolute** (`from kellu.common import paths`).
3. Importing a module has **no side effects** (no directories, no network, no registry writes).
4. Third-party packages are imported only by the layer that needs them, and heavy ones are
   optional extras (`[tui]`, `[local]`).
5. A package is created **when its milestone starts**. Empty packages for future features are
   not allowed; the architecture test fails on any directory that is not a declared layer.

## Where future features go

| Feature | Version | Placement |
|---|---|---|
| Decision cascade (rules, memory, LLM) | 0.2.0 to 0.4.0 | `core/cascade.py` |
| Agent loop | 0.2.0 | `core/agent.py` |
| OpenAI-compatible gateway, ACP agent mode | 0.3.0 | `frontends/gateway.py`, `frontends/acp.py` |
| Memory store (SQLite, optional pgvector) | 0.4.0 | new lower layer `memory/` (imports `common`) |
| MCP server | 0.5.0 | `frontends/mcp_server.py` |
| MCP client (agents and external tools) | 0.5.0 | `tools/mcp_client.py` |
| Local runtime detection | 0.5.0 | `providers/local.py` |
| Scope and grants | M3 | `security/scope.py`, `security/grants.py` |
| Append-only audit log | M3 | `security/audit.py` |
| Sandbox mounts derived from a grant, staging copy | M4 | `sandbox/policy.py`, `sandbox/staging.py` |
| Proposal contract and host-side apply | M5 | `tools/proposal.py`, `tools/apply.py` |
| Kate LSP server (hover, completion; later diagnostics, code actions) | M6, 0.2.0 | `frontends/lsp/` |

Protocol adapters (ACP, MCP, gateway) are **frontends** or **tools**, never top-level packages.

## Extensions (agents and MCP servers)

Agents such as a crawler or an OCR service, and any MCP server, are **untrusted extensions**.
They are reached through one generic client, run as sandboxed processes with a scrubbed
environment, are enabled by an explicit per-server allowlist, and every tool they expose needs
a declared risk level. The core contains no per-agent code.

## Trust boundary

The core is trusted: it holds keys and network access and enforces policy. Process-spawning
tools and extensions run in the sandbox and are untrusted. Model output, repository content and
web content are untrusted data. See [SECURITY.md](../SECURITY.md) and the design notes in [docs/design/](design/).

## Adding a module

1. Pick the lowest layer that can host it.
2. Add tests first, including negative tests for anything security-relevant.
3. Run `ruff check .`, `ruff format .`, `mypy` and `pytest`.
