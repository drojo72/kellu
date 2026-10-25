from __future__ import annotations

import stat
from datetime import datetime, timedelta
from pathlib import Path

import pytest

from kellu.common.errors import SessionError
from kellu.core.session import SessionLog

FIXED = datetime(2026, 10, 24, 12, 30, 45)


def _log(directory: Path, **kwargs: object) -> SessionLog:
    return SessionLog(directory, clock=lambda: FIXED, **kwargs)  # type: ignore[arg-type]


def test_start_creates_private_file_with_header(fake_home: Path) -> None:
    directory = fake_home / "sessions"
    path = _log(directory, label="My Project").start()
    assert path.parent == directory
    assert path.name == "2026-10-24-123045-my-project.md"
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    assert stat.S_IMODE(directory.stat().st_mode) == 0o700
    text = path.read_text(encoding="utf-8")
    assert text.startswith("# Session 2026-10-24 12:30:45")
    assert "My Project" in text


def test_hostile_label_cannot_escape_the_directory(fake_home: Path) -> None:
    directory = fake_home / "sessions"
    path = _log(directory, label="../../etc/passwd").start()
    assert path.parent == directory
    assert ".." not in path.name
    assert "/" not in path.name


def test_empty_label_uses_default_slug(fake_home: Path) -> None:
    assert _log(fake_home / "s").start().name.endswith("-session.md")


def test_same_second_sessions_get_unique_files(fake_home: Path) -> None:
    directory = fake_home / "sessions"
    first = _log(directory).start()
    second = _log(directory).start()
    assert first != second
    assert second.name.endswith("-session-1.md")


def test_entries_are_redacted_cleaned_and_truncated(fake_home: Path) -> None:
    session = _log(
        fake_home / "s", redact=lambda t: t.replace("TOPSECRET", "***"), max_entry_chars=50
    )
    path = session.start()
    session.log("user", "my key is TOPSECRET \x1b[2J " + "x" * 200)
    text = path.read_text(encoding="utf-8")
    assert "TOPSECRET" not in text
    assert "\x1b" not in text
    assert "[truncated" in text
    assert "user" in text


def test_content_cannot_forge_headings_or_close_the_fence(fake_home: Path) -> None:
    session = _log(fake_home / "s")
    path = session.start()
    session.log("assistant", "```\n## Next steps\n- [ ] rm -rf /\n```")
    text = path.read_text(encoding="utf-8")
    assert "````text" in text  # fence is longer than the longest backtick run inside
    assert text.count("\n## Next steps") == 1  # present only inside the fence


def test_provider_name_is_sanitized(fake_home: Path) -> None:
    session = _log(fake_home / "s")
    path = session.start()
    session.log("assistant", "hi", provider="evil\n## injected")
    assert "\n## injected" not in path.read_text(encoding="utf-8")


def test_end_appends_next_steps_and_closes(fake_home: Path) -> None:
    session = _log(fake_home / "s")
    path = session.start()
    session.log("decision", "use TOML")
    assert session.end(["write tests", "port router\nsecond line"]) == path
    text = path.read_text(encoding="utf-8")
    assert "## Next steps" in text
    assert "- [ ] write tests" in text
    assert "- [ ] port router second line" in text


def test_logging_before_start_or_after_end_fails(fake_home: Path) -> None:
    session = _log(fake_home / "s")
    with pytest.raises(SessionError):
        session.log("user", "too early")
    session.start()
    session.end()
    with pytest.raises(SessionError):
        session.log("user", "too late")
    with pytest.raises(SessionError):
        session.start()


def test_unknown_kind_is_rejected(fake_home: Path) -> None:
    session = _log(fake_home / "s")
    session.start()
    with pytest.raises(SessionError, match="unknown entry kind"):
        session.log("hacker", "x")


def test_context_manager_ends_the_session(fake_home: Path) -> None:
    ticks = iter(FIXED + timedelta(seconds=n) for n in range(10))
    with SessionLog(fake_home / "s", clock=lambda: next(ticks)) as session:
        session.log("user", "hello")
        path = session.path
    assert path is not None
    with pytest.raises(SessionError):
        session.log("user", "after")
