# Kellu

[English](README.md) · [Español](README.es.md)

**Secure and modular AI assistant for the Kate editor that thinks before spending tokens.**

> **Status: pre-0.1.0.** Rebuilt from scratch with a security-first design. An earlier prototype is being ported module by module. APIs, commands and configuration will change. Not yet installable from PyPI. Security model: see [SECURITY.md](SECURITY.md).

---

## What is Kellu?

Kellu brings AI assistance to [Kate](https://kate-editor.org/) without a heavy graphical stack. It is not just another LLM chat. A small, headless core decides **whether** a model is needed, redacts what leaves your machine, routes each task to the right provider with fallback, runs any process-spawning tool inside a sandbox, and talks to the editor through thin frontends.

## Principles

- **Security first, fail closed.** If a protection is unavailable (for example no sandbox backend), the dangerous capability is disabled instead of running unprotected.
- **Least privilege.** Keys live in the system keyring, separated by tiers. The model never sees secrets. Tools run with the minimum access required.
- **Untrusted by default.** Files, web pages and stored snippets are data, never instructions.
- **Decision cascade.** Rules, then memory, and the LLM as last resort. You can always force a model.
- **Modular and frugal.** Standard library first, optional extras, replaceable providers, stores and frontends.
- **Terminal first.** Validate the core before adding any GUI.
- **Adaptable.** Runs on a modest laptop with cloud providers, or on a workstation with local models.

## Version plan

| Version | Focus | Content |
|---|---|---|
| **0.1.0** | Secure base | Modular core, unified providers (OpenAI-compatible, Gemini and Gemma through Google; OpenRouter, DeepSeek and xAI Grok out of the box), keyring vault with tiers, redaction and deny globs, risk-based permissions, **sandbox** (bubblewrap), tools (files, sandboxed shell, git), terminal REPL, LSP for Kate (hover and completion), `kellu doctor` |
| **0.2.0** | Agent | Agent loop with tools and confirmations, cascade rules stage, context injection, LSP diagnostics and quick-fix suggestions |
| **0.3.0** | Interoperability | Local OpenAI-compatible gateway, ACP agent mode |
| **0.4.0** | Memory | Store of accepted solutions (SQLite by default, PostgreSQL + pgvector optional), memory stage |
| **0.5.0** | Adaptation | Local runtime detection (Ollama, LM Studio, llama.cpp), hardware profiles, MCP server and client |
| **Later** | | GUI via existing Kate plugins; Landlock, macOS and Windows sandbox backends |

0.1.0 **intentionally does not include an autonomous agent loop**: tools, permissions and the sandbox are validated separately before a model can trigger them.

## Architecture

```
 Terminal   Kate (LSP)   Gateway*   ACP*   MCP*          <- frontends
      \          |          |        |      /
 ========= Kellu core (trusted: keys, network) =========
  scope -> rules* -> memory* -> sanitizer -> router -> provider
                                            \-> tools (risk levels, permissions)
 =============================================================
         |                                   |
  sandbox (bubblewrap):               host side, with confirmation:
  shell, tests, formatters            git and gh writes

  * post-0.1.0
```

The core has no UI and no side effects on import. Frontends, providers and stores are replaceable adapters. Dependency direction: `frontends → core → tools → sandbox, security → common`, with `providers` and `projects` also below `core` (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)). A test enforces it.

## Security at a glance

- **Sandbox.** Process-spawning tools run under bubblewrap: no network by default, read-only system, project directory as the only writable path, scrubbed environment, no access to `~/.ssh`, `~/.gnupg` or the keyring bus.
- **Key tiers.** `provider` keys are available in the normal flow. `infra` and `db` secrets (cloud, DNS, databases) are not reachable by the assistant unless explicitly enabled with confirmation.
- **Risk levels.** Every tool is `read`, `write`, `destructive` or `network`. Destructive actions require typed confirmation. Git and GitHub writes run on the host, never inside the sandbox.
- **Data egress.** Deny globs (`.env`, keys, certificates) and secret redaction run before anything is sent to a provider. Per-project local-only mode is planned.
- **A Python virtualenv isolates dependencies, not security.** What restricts tools is the sandbox.

Details, threat model and limits: [SECURITY.md](SECURITY.md).

## Decision cascade (design, 0.2.0–0.4.0)

1. **Rules.** Deterministic handling that needs no model (stack detection, git status, known commands).
2. **Memory.** Looks up previously accepted solutions. Reused only if you accepted them or tests passed, similarity is above threshold, and they have not expired. Reused answers are labeled "from memory".
3. **LLM.** Routes by task type to the best provider, with fallback. Force a model with `--model` or an equivalent option.

## Projects and scopes

Projects are discovered under a configurable root and grouped into macro-project, repository and sub-project. Each scope can define rules, preferred providers, resource limits and a privacy mode (for example local-only).

## Agents and extensions

Kellu will speak the open protocols for agents and tools (ACP and MCP, between 0.3.0 and 0.5.0). Agents and MCP servers, whether first-party or third-party, are treated as **untrusted extensions**: they run as sandboxed processes with a scrubbed environment, are enabled only through an explicit per-server allowlist, and every tool they expose needs a declared risk level. There is no per-agent code inside the core; one generic client serves all of them.

Ideas under evaluation, as separate repositories and not part of 0.1.0:

- **kellu-crawler**: focused web crawler for project and research use
- **kellu-ocr**: text extraction (OCR) for old scanned PDFs and similar documents

Results would be delivered to the terminal or as files; delivery inside Kate depends on what its LSP client can display.

## Project layout

```
kellu/
├── LICENSE  README.md  README.es.md  SECURITY.md  CHANGELOG.md  pyproject.toml
├── src/kellu/
│   ├── __app__.py          # identity: name, version, keyring schema (single source)
│   ├── cli.py  doctor.py   # entry points: kellu, doctor, keys, lsp...
│   ├── common/             # errors, XDG paths (standard library only)
│   ├── security/           # vault, redaction, deny globs, permissions and policy
│   ├── sandbox/            # interface, bubblewrap backend, profiles
│   ├── providers/          # OpenAI-compatible, Google (local runtimes later)
│   ├── projects/           # discovery, stack detection, scopes
│   ├── tools/              # registry with mandatory risk level; files, shell, git
│   ├── core/               # config, router, context, sessions (no side effects on import)
│   └── frontends/          # REPL, LSP (later: gateway, ACP, MCP server)
├── tests/                  # unit/ and sandbox/
├── docs/                   # ARCHITECTURE.md and design notes
└── packaging/              # desktop entry
```

Packages are added **when their milestone starts**, not before: `memory/` (0.4.0, SQLite and optional pgvector), the decision cascade as `core/cascade.py` (0.2.0 to 0.4.0), and an MCP client under `tools/` (0.5.0). A test enforces that every directory is a declared layer and that layers only import downward.

## Quick start

> Commands describe the **target interface** and may change before 0.1.0.

Requirements (Linux first): Python 3.11+, `libsecret-tools` (`secret-tool`) and `bubblewrap`. On Debian/Ubuntu:

```
sudo apt install libsecret-tools bubblewrap
```

Some distributions restrict unprivileged user namespaces (e.g. Ubuntu 24.04); `kellu doctor` tells you whether the sandbox can run.

```
# Once published
pipx install kellu

# Development
git clone https://github.com/<your-user>/kellu.git
cd kellu
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"      # optional extras: [tui], [local], [dev]

kellu doctor                 # checks keyring, bubblewrap and user namespaces
kellu keys set openrouter    # stored in your keyring, never printed
kellu                        # terminal assistant
```

If you have secrets under a previous keyring schema, `kellu keys migrate --dry-run` lists names (never values) and the copy never deletes the originals.

### Use it inside Kate (LSP)

In Kate: *Settings → Configure Kate → LSP Client → User Server Settings*. Example:

```json
{
  "servers": {
    "python": {
      "command": ["kellu", "lsp"],
      "highlightingModeRegex": "^Python$"
    }
  }
}
```

In 0.1.0 Kellu’s LSP server provides **hover** and **completion**; diagnostics and quick-fix suggestions arrive with the agent work in 0.2.0. Chat, diff previews and ghost text need other frontends (see [docs/design/lsp-capabilities.md](docs/design/lsp-capabilities.md)). Check Kate’s LSP client documentation for the exact format on your version.

## Configuration

| Path | Purpose |
|---|---|
| `~/.config/kellu/` | Providers, routing, scopes, sandbox policy |
| `~/.local/state/kellu/` | Session logs, history (redacted, mode 0700) |
| `~/.cache/kellu/` | Caches |

Configuration is a single TOML file, `~/.config/kellu/config.toml`, merged over packaged defaults. It never holds secrets (use the keyring), endpoints must use HTTPS (plain HTTP only for localhost), unknown keys are errors, and the file must be owned by you and not writable by group or others. Inspect it with `kellu config path` and `kellu config check`.

```toml
[routing.code]
primary = "deepseek"
backup = ["openrouter"]

[routing.general]
primary = "google"
backup = ["deepseek", "openrouter"]
```

## Hardware profiles (design target)

| Profile | Target | What runs |
|---|---|---|
| `minimal` | any machine | core + terminal, cloud providers, SQLite |
| `standard` | typical laptop | plus LSP and MCP, small local model for completion |
| `full` | ample RAM or GPU | plus Kate plugin integration, larger local models, PostgreSQL + pgvector |

## Milestones toward 0.1.0

- [x] **M0** Repository, `src/` layout, `pyproject.toml`, XDG paths, CI, tests, license, `SECURITY.md`
- [x] **M1** Config with packaged defaults, errors, logging, sessions; no side effects on import
- [x] **M2** Providers: OpenAI-compatible client (streaming, tools), Google, router with timeouts and fallback
- [ ] **M3** `kellu keys` with tiers and migration, scrubbed environment, deny globs, redaction v1, risk-based permissions
- [ ] **M4** Sandbox: interface, bubblewrap backend, policies, `kellu doctor`, escape tests
- [ ] **M5** Tools with generated schemas and risk levels: files, sandboxed shell, read-only git; git/gh writes on host with confirmation
- [ ] **M6** Terminal REPL on top of the core; LSP for Kate (hover, completion)
- [ ] **M7** Documentation and 0.1.0 tag

## Related projects

Kellu is independent, but is designed to interoperate with other AI efforts for Kate rather than replace them:

- [KateAI](https://github.com/ObiWindu/KateAI) — Kate plugin with a full agent GUI experience.
- [kate-agents](https://github.com/Mte90/kate-agents) — Kate plugin that brings Zed-style agents into the editor.

Interoperability is a goal, not an existing integration. No endorsement by their authors is implied.

## About the name

*Kellu* means "helper" in Mapudungun, the language of the Mapuche people. The name reflects the project’s purpose: an assistant that helps, quietly, inside the tools you already use.

## Contributing

The project is young and the architecture is still moving, so open an issue before large changes. Especially welcome: security review, sanitization, provider abstractions, LSP integration, and data schemas for development-oriented memory. **Report vulnerabilities privately**, as described in [SECURITY.md](SECURITY.md).

## License

MIT. See [LICENSE](LICENSE).

## Disclaimer

Kate is a text editor developed by the KDE community. Kellu is an independent project and is not affiliated with, sponsored by, or endorsed by KDE. AI-generated output can be incorrect: review suggestions before applying them.
