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

> **Status: confirmed by the user on 2026-11-24.** README, README.es, ARCHITECTURE, SECURITY and
> the design documents use this numbering. Crate and library names below are candidates: check
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

### 0.2.0 Control (providers, models, tokens and cost)

The most valuable step after 0.1.0, because it is what hurt in real use.

**Guiding rule: a token spent is the last resort.** A request reaches a model only after everything
cheaper has had its chance: the AI switch (off means nothing is sent), the exact-match cache,
the budget planner (which may refuse), and only then the router, whose chain starts with the
cheapest suitable provider. To keep 0.2.0 small, the work is split in a core and a tail (0.2.x).

**0.2.0 core:** model catalog, enable and disable, provider parameters, router health, AI switch,
LSP robustness, exact-match cache, budget planner, usage ledger with caps, `kellu usage`, prompt
presets. **0.2.x tail (only if the core proves stable):** `kellu bench`, the `fastest` and
`quality` policies, the golden set.

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
- **Token and cost management** (the goal: best quality per unit of cost, and as much of the
  budget as possible left for the answer). Details in the next section.
- **AI switch:** `[ai] enabled`, with overrides per language id, per file glob and per project;
  off means nothing leaves the machine and the server answers with local data only.
- **LSP robustness:** a worker thread for model calls, `$/cancelRequest`, a per-request timeout,
  and a visible message when the model fails.
- Documents: SECURITY (catalog responses are untrusted data: names are sanitized, never executed
  or placed in prompts), config reference.

#### Token and cost management (0.2.0)

**1. Model profile.** Each model in the catalog gets a profile: context window and maximum output
(from the catalog where the service reports them, otherwise set by the user), price per million
tokens (OpenRouter's model list includes prices; for other services they are set in the config and
flagged `unknown` until then; verify each service when the milestone starts), whether it reasons,
and an **effective context**: the largest prompt size at which answers stay reliable. The effective
context is not the advertised window; long prompts degrade quality well before the limit, so the
default is conservative (a fraction of the window, a heuristic to be replaced by measurement).

**2. Budget planner** (in `kellu-core::assist`, the single egress point). Every request is planned
before it is sent: `prompt budget = min(effective context, window - reserve)`, where the reserve is
the output you want plus headroom for hidden reasoning (the failure seen in 0.1.0). If the prompt
does not fit, it is compacted in a fixed order, cheapest loss first: (a) drop the oldest chat turns,
(b) collapse blank runs and trailing spaces (loses nothing; never the line being typed),
(c) narrow the code window around the cursor, both sides together, (d) refuse with a clear message rather than send a truncated task that invites a made-up
answer. No model call is used to compact: it must not spend tokens to save tokens.

**3. Token counting.** There is no exact tokenizer for every service, so Kellu estimates (characters
per token, per language: Spanish and German cost more than English) and **calibrates** the ratio per
model from the `usage` each response reports. The estimate only plans; the service's own count is
what the ledger records.

**4. Ledger and policy** (core: `cheapest` and `balanced`; the rest in the tail). A local usage ledger (provider, model, task, prompt and answer tokens,
latency, outcome, estimated cost; never any text; mode 0600; rotated; the cost is shown in each service's own unit, for people to
read, with no currency conversion) feeds `kellu usage` (by day,
provider, model, route). Per route, a **policy** picks the order of the chain: `cheapest`,
`fastest`, `balanced` or `quality`, with limits: maximum cost per request, daily and monthly caps,
and what happens at the cap (fall to a cheaper model, fall to local only, or stop). Paid models are
shown as paid and never used for a route unless allowed.

**5. Calibration command, explicit and capped (0.2.x tail).** `kellu bench <provider/model>` measures, with a
token cap you confirm before it runs: latency and tokens per second, and the **effective
context** with a needle-in-context test at growing prompt sizes (the first size where recall drops
is the safe limit; Kellu stores it in the profile). The user may add a small golden set of their own
questions to compare models on their real tasks. This is how "the window of first-quality answers
before it starts making things up" is estimated: measured per model, never assumed, and it costs
tokens, so it never runs on its own.

**6. Token-saving prompt presets** (what the roadmap calls skills). Short, fixed instructions per
task kind (explain, complete, review, summarise, translate) that ask for a bounded, plain answer
and set the output reserve; chosen by the router's task classifier. They are Kellu configuration,
versioned with the code and tested for injection like the rest of the prompt (a preset never
contains user text), not model-side skills.

What this cannot do: no number guarantees the absence of fabricated answers. The effective context
lowers the risk, the planner refuses what does not fit, and the answers keep their "not verified"
nature; correctness still needs the user's review and, in 0.5.0, tests.

### 0.3.0 Local (project index, prediction, rules)

Goal: fewer tokens and faster hover without changing the trust model.

1. **Exact-match cache (moved to the 0.2.0 core; in-memory version done, persistence pending).** Key: hash of the redacted messages, task, route and sampling settings (the chain, since the provider is chosen by the router). Stores only
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

0. Decided (2026-11-24): the ledger shows cost in each service's own unit, informational only.

1. Confirm the order above (it moves the agent from 0.2.0 to 0.5.0 and memory from 0.4.0 to 0.6.0).
2. Whether the 0.3.0 cache may persist between sessions by default (recommended: yes, redacted,
   0600, with a TTL and `kellu cache clear`) or must be opt-in.
3. Results of your multi-server test in Kate, before 0.4.0 starts.
