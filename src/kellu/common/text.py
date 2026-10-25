"""Small text helpers shared by logging and session code. Standard library only."""

from __future__ import annotations

import unicodedata
from collections.abc import Callable

# A redactor maps text to the same text with secrets removed. The real implementation lives in
# ``kellu.security`` (milestone M3) and is injected; until then ``identity_redactor`` is used.
Redactor = Callable[[str], str]


def identity_redactor(text: str) -> str:
    """Return ``text`` unchanged (placeholder until the redaction engine exists)."""
    return text


def strip_control_chars(text: str) -> str:
    """Remove control and format characters, keeping only newline and tab.

    This blocks terminal-escape injection (ESC sequences, carriage returns) and bidirectional
    spoofing when logs are shown with ``cat`` or ``less``.
    """
    return "".join(
        ch for ch in text if ch in "\n\t" or not unicodedata.category(ch).startswith("C")
    )


def truncate(text: str, limit: int) -> str:
    """Cut ``text`` to ``limit`` characters and say how much was dropped."""
    if limit < 1:
        raise ValueError("limit must be positive")
    if len(text) <= limit:
        return text
    return f"{text[:limit]}... [truncated {len(text) - limit} chars]"
