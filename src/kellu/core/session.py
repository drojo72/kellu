"""Session logs: append-only Markdown files, redacted, size-limited and private (mode 0600).

Every entry goes through the same pipeline: redact secrets, strip control characters, truncate.
Content is written inside a code fence longer than any backtick run it contains, so it cannot
forge headings or close the fence. File names come from a sanitized slug, never from raw input.
"""

from __future__ import annotations

import re
from collections.abc import Callable, Sequence
from datetime import datetime
from io import TextIOWrapper
from pathlib import Path
from types import TracebackType

from kellu.common import paths
from kellu.common.errors import SessionError
from kellu.common.text import Redactor, identity_redactor, strip_control_chars, truncate

KINDS = frozenset({"user", "assistant", "tool", "decision", "error"})
_SLUG_RE = re.compile(r"[^a-z0-9]+")
_PROVIDER_RE = re.compile(r"[^A-Za-z0-9_.-]")
_MAX_SLUG = 40
_MAX_ATTEMPTS = 100


def _slugify(label: str | None) -> str:
    slug = _SLUG_RE.sub("-", (label or "").lower()).strip("-")[:_MAX_SLUG].strip("-")
    return slug or "session"


def _fence_for(text: str) -> str:
    longest = max((len(match.group()) for match in re.finditer(r"`+", text)), default=0)
    return "`" * max(3, longest + 1)


class SessionLog:
    """One conversation session written to a Markdown file."""

    def __init__(
        self,
        directory: Path,
        *,
        label: str | None = None,
        redact: Redactor = identity_redactor,
        max_entry_chars: int = 2000,
        clock: Callable[[], datetime] = datetime.now,
    ) -> None:
        self._directory = directory
        self._label = label
        self._redact = redact
        self._max_chars = max_entry_chars
        self._clock = clock
        self._file: TextIOWrapper | None = None
        self._state = "new"  # new -> active -> ended
        self.path: Path | None = None

    # ------------------------------------------------------------- lifecycle ----
    def start(self) -> Path:
        """Create the session file and write its header."""
        if self._state != "new":
            raise SessionError("session was already started")
        paths.ensure_private_dir(self._directory)
        started = self._clock()
        stem = f"{started:%Y-%m-%d-%H%M%S}-{_slugify(self._label)}"
        handle, path = self._open_unique(stem)
        self._file, self.path, self._state = handle, path, "active"
        header = f"# Session {started:%Y-%m-%d %H:%M:%S}\n\n"
        if self._label:
            header += f"- **Context**: {self._clean(self._label, 100, single_line=True)}\n\n"
        self._write(header)
        return path

    def end(self, next_steps: Sequence[str] = ()) -> Path:
        """Append the next steps (if any) and close the file."""
        self._require_active()
        if next_steps:
            steps = "\n".join(
                f"- [ ] {self._clean(step, 200, single_line=True)}" for step in next_steps
            )
            self._write(f"## Next steps\n\n{steps}\n")
        if self._file is None or self.path is None:  # pragma: no cover - guarded by state
            raise SessionError("session is not active")
        self._file.close()
        self._file, self._state = None, "ended"
        return self.path

    def __enter__(self) -> SessionLog:
        self.start()
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        if self._state == "active":
            self.end()

    # --------------------------------------------------------------- entries ----
    def log(self, kind: str, content: str, *, provider: str | None = None) -> None:
        """Append one entry. ``kind`` must be one of :data:`KINDS`."""
        self._require_active()
        if kind not in KINDS:
            raise SessionError(f"unknown entry kind: {kind!r}")
        body = self._clean(content, self._max_chars)
        who = kind
        if provider:
            who = f"{kind} ({_PROVIDER_RE.sub('?', provider)[:40]})"
        fence = _fence_for(body)
        stamp = self._clock()
        self._write(f"### {stamp:%H:%M:%S} · {who}\n\n{fence}text\n{body}\n{fence}\n\n")

    # --------------------------------------------------------------- helpers ----
    def _clean(self, text: str, limit: int, *, single_line: bool = False) -> str:
        """Redact first (so a secret cut in half is never exposed), then sanitize and truncate."""
        cleaned = strip_control_chars(self._redact(text))
        if single_line:
            cleaned = " ".join(cleaned.split())
        return truncate(cleaned, limit)

    def _open_unique(self, stem: str) -> tuple[TextIOWrapper, Path]:
        for attempt in range(_MAX_ATTEMPTS):
            name = f"{stem}.md" if attempt == 0 else f"{stem}-{attempt}.md"
            path = self._directory / name
            try:
                return paths.open_private_append(path, new_only=True), path
            except FileExistsError:
                continue
        raise SessionError("could not allocate a unique session file")

    def _require_active(self) -> None:
        if self._state != "active":
            raise SessionError("session is not active")

    def _write(self, text: str) -> None:
        if self._file is None:  # pragma: no cover - guarded by state
            raise SessionError("session is not active")
        self._file.write(text)
        self._file.flush()
