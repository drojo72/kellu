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

# Roadmap to 1.0

> **Status: proposal (2026-11-13), not yet confirmed.** README, ARCHITECTURE and SECURITY still
> carry the earlier numbering (0.2.0 agent, 0.3.0 gateway, 0.4.0 memory, 0.5.0 adaptation). They
> are updated once this order is confirmed. Crate and library names below are candidates: check
> versions, licences and maintenance when the milestone starts, not before.

## Where we are

0.1.0 is functionally done (M0 to M6, M7 in progress): hardened providers and router, keyring vault,
redaction, deny globs, scope and grants, bubblewrap sandbox, tools, terminal chat, and a Kate LSP
server with hover and completion (tested in a real Kate on `.rs` and `.py`; about 4 MB RSS).

What the first real-world tests showed, and the plan below fixes:

| Finding | Consequence |
|---|---|
| Packaged primaries failed (no balance, 503, retired or inaccessible model) and every hover paid for the failure first | Providers need health memory, a real model catalog and per-route control |
| A reasoning model spent the whole token budget and answered nothing | Per-provider parameters (token cap, reasoning off or low), and empty answers must fall through to the next provider |
| The model call blocks the single LSP thread | Hover needs a worker, cancellation and a timeout that Kate can survive |
| Every hover and completion goes to a paid model | A local layer (cache, project index, rules) and an explicit AI on/off |
| A failure is visible only in a private log | `window/showMessage` or a status diagnostic |

## Proposed order

### 0.2.0 Control (providers, models, switches)

The most valuable step after 0.1.0, because it is what hurt in real use.

- **Model catalog.** `kellu models refresh [provider]` asks each service for the models the
  account can really use (`GET /models` for OpenAI-compatible services, the model list of Google),
  and caches the result under `~/.cache/kellu/`. Never at startup or build (no effects at start);
  always an explicit command. `kellu models list` shows it, flags paid or unknown cost, and
  `kellu models use <route> <provider>/<model>` writes the choice to the user config.
- **Enable or disable** providers and single models (`enabled = false`), per user and per project
  (a project can only turn things off, as with `[lsp]`).
- **Advanced provider settings:** `max_tokens` cap, `reasoning = "off" | "low" | "default"` where
  the API supports it, extra request parameters from an allowlist, rate limit, retry and timeout
  per provider, a local token and spend counter with an optional daily cap, custom OpenAI-compatible
  endpoints (the config already allows plain HTTP for localhost, so Ollama, llama.cpp and vLLM
  should work; not verified).
- **Router health:** remember a failing provider for a cooldown (circuit breaker) so a dead
  primary costs one failure, not one per request; an empty answer counts as a failure and falls
  through; `kellu providers status` shows state.
- **AI switch:** `[ai] enabled`, with overrides per language id, per file glob and per project;
  off means nothing leaves the machine and the server answers with local data only.
- **LSP robustness:** a worker thread for model calls, `$/cancelRequest`, a per-request timeout,
  and a visible message when the model fails.
- Documents: SECURITY (catalog responses are untrusted data: names are sanitized, never executed
  or placed in prompts), config reference.

### 0.3.0 Local (cache, project index, rules)

Goal: fewer tokens and faster hover without changing the trust model.

1. **Exact-match cache.** Key: hash of the redacted window, task, provider and model. Stores only
   redacted text, mode 0600, with a TTL and a size cap, per project, honouring deny globs, and
   `kellu cache clear`. Cuts repeated hovers and completions to zero tokens.
2. **Project symbol index** (no model): definitions and their doc comments or signatures,
   extracted by simple per-language scanners. Answers hover for known symbols directly, labelled as
   local, and gives completion candidates.
3. **Local prediction:** prefix completion from identifiers of the open documents and the index
   (frequency and recency). No model, no network, instant.
4. **Cascade stages 1 and 2** (rules, then local) in `kellu-core`; the model is stage 3.

Storage is the open decision. My recommendation: **start without a database** (in-memory index per
session plus a small persistent cache file); introduce an embedded store only when 0.6.0 memory
needs queries. Candidates then: SQLite through `rusqlite` (C code compiled in: justify and audit)
or a pure-Rust key-value store such as `redb`. Decide with measurements (memory, start time) when
0.3.0 begins. Embeddings and `pgvector` stay in the memory milestone.

### 0.4.0 Prose (LanguageTool and Markdown)

- LSP **diagnostics** infrastructure (`publishDiagnostics`, positions in UTF-16, debounce) and
  quick fixes from LanguageTool replacements.
- **Markdown regions module:** front matter, code fences with their language, inline code,
  mermaid, YAML and text; decides the language per file, section or block, and masks code so it
  is never checked as prose. Positions must survive redaction.
- **LanguageTool backend over HTTPS** (own server or premium cloud), never on every keystroke and
  never the public API for automatic use (its terms forbid it). Languages: es, en, de, fr with
  LanguageTool; it and el with the model as a second opinion; he and la through Kate's own checker
  plus a model code action.
- Measure first: RSS of LanguageTool with a normal `.md`, and whether Kate runs several servers per
  document (you test this), which decides coexistence with LTeX+.
- Premium key: needs a keyring level other than `ai_*` (the provider level only accepts `ai_*`).
- Own Rust implementation only if measurements justify it; C only after analysis, as a separate
  sandboxed process.

### 0.5.0 Agent

Agent loop with tools and confirmations, context injection, model-based diagnostics and code
actions (one per hunk plus "apply all"), `kellu-projects` (discovery, scopes, privacy mode such as
local-only). The model still proposes and only the core applies.

### 0.6.0 Memory

`kellu-memory`: accepted solutions with acceptance criteria, expiry and a "from memory" label
(cascade stage 2, extended); similarity search; optional PostgreSQL and `pgvector`.

### 0.7.0 Interoperability

Local OpenAI-compatible gateway, ACP agent mode. Same risk, sandbox and audit rules as tools.

### 0.8.0 Adaptation

Local runtime detection (Ollama, LM Studio, llama.cpp), hardware profiles, MCP server and client
(extensions are untrusted: sandboxed, per-server allowlist, declared risk per tool).

### 0.9.0 Hardening and packaging

External security review, fuzzing of the LSP framing and the config parser, mutation testing across
the workspace, reproducible builds, packages (`.deb`, Flatpak or tarball: decide then), man pages,
compatibility matrix for Kate versions, translated documentation reviewed.

### 1.0.0 Criteria (all must hold)

1. Stable contracts, covered by tests and the changelog: config keys, CLI commands, LSP
   capabilities, on-disk formats (audit log, cache, memory), with a versioned migration path.
2. No known open issue in the threat model (SECURITY.md); the review of 0.9.0 closed.
3. Installable by a user who is not the maintainer, with `kellu doctor` explaining any missing piece.
4. Works on the Kate versions in the compatibility matrix; steady-state memory of `kellu lsp`
   stays small and is measured in CI.
5. Every provider in the packaged defaults was verified with a real call on release day, and none
   is on by default without a key.

## Why this order

- **Control before Local:** a cache or index on top of an unreliable router hides problems.
- **Local before Prose and Agent:** it lowers cost for everything that follows, and the cascade
  stage it adds is reused by both.
- **Prose before Agent:** it is the feature you asked for, it is read-only (diagnostics), and it
  builds the diagnostics path that the agent's quick fixes reuse.
- **Agent later than first planned:** it is the riskiest piece; it gains from the catalog, health,
  local layer and scopes being in place, and 0.1.0 already validated tools and sandbox apart.

## Still open (user decides)

1. Confirm the order above (it moves the agent from 0.2.0 to 0.5.0 and memory from 0.4.0 to 0.6.0).
2. Whether the 0.3.0 cache may persist between sessions by default (recommended: yes, redacted,
   0600, with a TTL and `kellu cache clear`) or must be opt-in.
3. Results of your multi-server test in Kate, before 0.4.0 starts.
