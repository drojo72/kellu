from __future__ import annotations

import pytest

from kellu.common.text import identity_redactor, strip_control_chars, truncate


def test_identity_redactor_returns_text_unchanged() -> None:
    assert identity_redactor("abc") == "abc"


def test_strip_control_chars_keeps_newline_and_tab() -> None:
    assert strip_control_chars("a\tb\nc") == "a\tb\nc"


def test_strip_control_chars_removes_escape_sequences_and_carriage_returns() -> None:
    assert strip_control_chars("ok\x1b[31mred\x1b[0m\rhidden") == "ok[31mred[0mhidden"


def test_strip_control_chars_removes_bidi_override() -> None:
    assert strip_control_chars("abc\u202edef") == "abcdef"


def test_truncate_leaves_short_text_alone() -> None:
    assert truncate("hello", 10) == "hello"


def test_truncate_reports_dropped_characters() -> None:
    assert truncate("a" * 15, 10) == "a" * 10 + "... [truncated 5 chars]"


def test_truncate_rejects_non_positive_limit() -> None:
    with pytest.raises(ValueError):
        truncate("x", 0)
