# Permissions and sandbox design

Status: design for milestones M3 to M5. It keeps the practical parts of an external proposal
(propose, approve, apply; hashes; audit) and drops what does not fit Kellu: a read-write sandbox
for the model, overlayfs, and Kate-internal APIs that an external process cannot call.

## Principles

1. **The real project is never writable from any sandbox.**
2. Writes happen only when the **core applies an approved, validated, hash-checked proposal**.
3. One `Grant` object drives both the path policy and the sandbox mounts. No parallel configs.
4. Fail closed: any validation failure applies nothing.

## Who may do what

| Component | Trust | Reads project | Writes project | Network |
|---|---|---|---|---|
| Kellu core (its own venv or pipx) | Trusted | Through the scope policy | Only via `apply_proposal` | Provider calls only |
| Model provider | External | Only the redacted context the core sends | Never | n/a |
| Sandboxed tools | Untrusted | Read-only tree or staging copy | Never (staging only) | None by default |
| Extensions (MCP servers, agents) | Untrusted | As sandboxed tools | Never | Only if granted |

Environments: Kellu's venv is trusted and never runs project or model-generated code. The sandbox
runtime is the system toolchain mounted read-only, plus the project's own `.venv` read-only. There
is no `pip` inside the sandbox. A venv is not a security boundary; the sandbox is.

## Scope

- **Root** comes from the user (`--project`, or the LSP `rootUri`). Refuse `/`, `$HOME` and any
  ancestor of `$HOME`. Kate falls back to the home directory as LSP root when it cannot find a
  project, so this check is mandatory. Require a marker (`.git`, `.kateproject`, `pyproject.toml`,
  `package.json`, `Cargo.toml`, ...) or an explicit confirmation.
- **File list:** `git ls-files` plus untracked, non-ignored files when the root is a git repository;
  otherwise a bounded walk. `.gitignore` is a hint to keep build output out of the context, not a
  security boundary.
- **Always denied inside the scope:** `.env*`, `*.pem`, `*.key`, `id_*`, `credentials*`, `*.token`,
  and `.git/` internals. Hidden files are denied except an allowlist (`.gitignore`,
  `.editorconfig`, `.github/`, `.kateproject`). Users can add denies; built-in ones cannot be
  removed silently.
- **Resolution:** resolve, require containment in the root, and open by walking directory
  descriptors with `O_NOFOLLOW`. Never check a path and then open it by name.

## Grant

```
Grant
  root:     Path
  read:     frozenset[relative path]          # scope listing minus denies
  write:    mapping relative path -> sha256   # frozen list; empty unless a proposal is approved
  network:  bool                              # default False
  expires:  monotonic deadline
```

Read access is granted when the user opens the project. Write access is never standing: it is
granted per proposal and only for the approved items.

## Proposal, approval, apply

**Contract** (plain JSON, validated by hand with the standard library): `proposal_id`, `root`, and
`items[]` with `id`, `tool`, `path` (relative), `operation` (`modify`, `create`, `delete`),
`base_hash` (required for modify and delete), `diff`, `rationale`. A proposal is immutable once
emitted.

**Validation before display:** path inside the scope and not denied, containment after resolution,
operation coherent with the tool, destination directory inside the scope for `create`. Invalid
items are never shown as approvable.

**Decision, chosen by the user:**

| Choice | Effect |
|---|---|
| Deny | Nothing is applied. |
| Partial | Only the selected hunks. |
| Full | All valid items of this proposal (never anything outside it). |

The frozen list is the approved item ids plus their hashes. `delete` needs typed confirmation.

**Apply, by the core only:** recheck hashes on open descriptors, revalidate the scope, apply only
approved hunks via temporary file plus atomic rename in the same directory, keep a backup of the
originals in the state directory (0600), and write the audit entry. If anything fails, apply
nothing. When the file is open in Kate, the LSP frontend applies the change as a `WorkspaceEdit`
with the document version instead of writing to disk, so unsaved edits are not overwritten and the
change is undoable.

## Sandbox

- Backend: bubblewrap. Build `argv` as a list and never use a shell. Mounts come from
  `sandbox/policy.py`, derived from the `Grant`; a test asserts that the mounts equal the grant.
- **Mount invariant:** bubblewrap never creates mountpoints inside a project tree. The project is
  always a leaf mount (read-only, or the staging directory) at a fixed destination on a fresh
  tmpfs root. No nested binds inside it and no symlinks in destination paths. This is what keeps
  CVE-2026-87766 (setup-time symlink traversal when bwrap creates files in attacker-controlled
  content) from applying.
- **Version policy B:** versions before 0.12.0 are allowed with a warning from `kellu doctor`.
  Setting `sandbox.require_patched_bwrap = true` makes it strict. Revisit when distributions catch
  up. Ubuntu 24.04 ships 0.9.0; its changelog shows the fix was backported and then dropped in a
  later update because of a Flatpak regression.
- **Defaults:** `--unshare-all` (no network), `--clearenv` plus `PATH`, `HOME`, `TMPDIR`,
  `--die-with-parent`, `--new-session`, read-only system, tmpfs for `HOME` and `/tmp`, resource
  limits (CPU time, address space, processes, file size), wall-clock timeout, process-group kill
  and an output size cap.
- **Tools that write** (formatters, generators) run on a **staging copy** of the in-scope files
  (reflink when available, size capped). The core diffs staging against the original; the result
  re-enters as a proposal and goes through the same approval. Changes the proposal did not announce
  are reported, never applied. Tested behavior: with the project mounted read-only, writes and new
  files fail, host symlinks dangle, the environment is empty and only loopback exists; with a
  staging copy, the original stays intact and the diff reveals extra files.

## Audit

Append-only JSONL in the state directory (mode 0600), one line per transition: `proposed`,
`approved` or `rejected`, `applied` or `failed`. Fields: timestamp, proposal id, resolved path,
permission, origin, validation results, hashes, state. Each line carries the hash of the previous
one for tamper evidence. Viewer: `kellu audit`.

## Deferred

XDG FileChooser portal (an explicit path grant with confirmation is enough for now), Landlock as a
fallback where user namespaces are restricted, overlayfs (needs bubblewrap 0.11 or newer and kernel
support), and signed persistent grants.

## Decisions

- Frozen list lives in memory; its hash goes to the audit log.
- Built-in denies are fixed; users may add.
- Timeouts by tool class, configurable per scope in TOML.
- Audit format: JSONL.
- A file edited between proposal and apply makes its items stale; no automatic re-analysis.
