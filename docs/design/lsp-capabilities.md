<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
</p>
<p align="center">
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# What Kellu can do through Kate's LSP client

Sources: the Kate handbook page "LSP Client Plugin" (docs.kde.org, KF6) and the kate-agents README.
Items marked **(verify)** are not documented and must be tested on the target Kate version.

## Documented client features

Go to definition, declaration, type definition and implementation; find references; document
highlight; hover (mouse or manual request); formatting and range formatting; rename; Quick Fix
(`codeAction` plus `executeCommand`, for a diagnostic at the cursor position or line); completion
with documentation and signature help; on-type formatting; incremental sync; diagnostics (tab,
highlights, marks, suppressions); document symbol outline; workspace symbol search; per-project
server configuration through the `lspclient` entry of `.kateproject`; one server instance per
(root, server type). Kate does not run code actions on save.

## Kellu services over LSP

| Service | Mechanism | Version | Notes |
|---|---|---|---|
| AI hover explanation | `textDocument/hover` | 0.1.0 | Keep answers short and cached; the user can request it manually |
| AI completion | `textDocument/completion` | 0.1.0 | One multi-line item with a `textEdit`; a dropdown, not ghost text; no trigger characters, so it runs only on manual request |
| Review findings | `publishDiagnostics` (info or hint, source `kellu`) | 0.2.0 | On request or on save; Kate supports suppressions |
| Suggested edits | `textDocument/codeAction` returning an `edit` | 0.2.0 | One action per hunk plus "Apply all": picking hunks is partial approval, "Apply all" is full, ignoring is deny. Titles must describe the change because there is no diff preview. Whether actions appear without a diagnostic at the position is **(verify)**; anchor them on an informational diagnostic if not |
| Per-project settings | `initializationOptions`, `workspace/didChangeConfiguration` | 0.1.0 | Comes from `.kateproject`: untrusted, may only tighten policy |
| Messages and progress | `window/showMessage`, `$/progress` | 0.2.0 | Shown in Kate's Output tab; behavior **(verify)** |

## What needs another approach

| Need | Why LSP is not enough | Approach |
|---|---|---|
| Chat, multi-turn, streaming | No UI for it | Terminal frontend, for example in Kate's embedded terminal view |
| Agent steps and tool-call display | No UI | Terminal; later an ACP client |
| Diff preview and rich approval | Quick Fix shows titles only | Terminal diff view; open the location in Kate from the terminal; GUI through KateAI over ACP |
| Ghost text, inline completion | Not among the documented client features | A Kate plugin (KTextEditor); Kellu reaches it through the gateway or ACP |
| Auto-fix on save | Kate does not run code actions on save | Explicit actions only |
| Custom menu entries and panels | Fixed menu | Terminal or plugin |
| Project file list from Kate | The Project plugin is not exposed over LSP | Kellu computes it (git) |
| Reuse Kate's LSP from another plugin | Not exposed (reported by kate-agents) | Not applicable |

Between frontends, share state through files in the state directory (proposals and the audit log)
in 0.1.0. A shared local daemon can come later if several LSP instances become a problem.

## Trust notes

- **Root:** when Kate cannot determine a root it falls back to the home directory. Kellu rejects
  `$HOME`, its ancestors and `/` as a scope.
- **Open documents:** Kate sends the full text of every open document, including `.env` if the user
  opens it. Deny globs apply to documents received over LSP; denied documents are never forwarded to
  a provider or kept.
- **`.kateproject` is repository-controlled.** Settings from it may only tighten Kellu's policy
  (for example local-only mode), never relax deny globs, sandbox profile or endpoints. The handbook
  says the `lspclient` entry overrides the server configuration; opening an untrusted project with
  LSP enabled therefore deserves the same care as running its configuration **(verify the
  command-override behavior)**.
- **Edits:** only edits from a validated proposal, only inside the scope, with document versions.
- **Instances:** Kate starts one server per (root, language). Keep `kellu lsp` light and load the
  core lazily.
