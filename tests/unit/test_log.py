from __future__ import annotations

import logging
import stat
from pathlib import Path

import pytest

from kellu.common.errors import ConfigError
from kellu.core import log


@pytest.fixture(autouse=True)
def _clean_logger() -> object:
    yield
    logger = logging.getLogger("kellu")
    for handler in list(logger.handlers):
        logger.removeHandler(handler)
        handler.close()


def _mode(path: Path) -> int:
    return stat.S_IMODE(path.stat().st_mode)


def test_creates_private_directory_and_file(fake_home: Path) -> None:
    state = fake_home / "state"
    logger = log.configure_logging(state)
    logger.info("hello")
    assert _mode(state) == 0o700
    assert _mode(state / log.LOG_FILENAME) == 0o600
    assert "hello" in (state / log.LOG_FILENAME).read_text(encoding="utf-8")


def test_importing_the_module_creates_nothing(fake_home: Path) -> None:
    assert list(fake_home.iterdir()) == []


def test_redaction_applies_to_messages_and_exceptions(fake_home: Path) -> None:
    state = fake_home / "state"
    logger = log.configure_logging(state, redact=lambda text: text.replace("TOPSECRET", "***"))
    logger.info("token is %s", "TOPSECRET")
    try:
        raise RuntimeError("failed with TOPSECRET")
    except RuntimeError:
        logger.exception("boom")
    content = (state / log.LOG_FILENAME).read_text(encoding="utf-8")
    assert "TOPSECRET" not in content
    assert "***" in content


def test_control_characters_are_stripped(fake_home: Path) -> None:
    state = fake_home / "state"
    logger = log.configure_logging(state)
    logger.info("evil \x1b[2J payload\r")
    content = (state / log.LOG_FILENAME).read_text(encoding="utf-8")
    assert "\x1b" not in content
    assert "\r" not in content


def test_configuring_twice_does_not_duplicate_output(fake_home: Path) -> None:
    state = fake_home / "state"
    log.configure_logging(state)
    logger = log.configure_logging(state)
    logger.info("once")
    content = (state / log.LOG_FILENAME).read_text(encoding="utf-8")
    assert content.count("once") == 1


def test_rotated_files_stay_private(fake_home: Path) -> None:
    state = fake_home / "state"
    logger = log.configure_logging(state, max_bytes=200, backups=2)
    for index in range(30):
        logger.info("line %03d padding padding padding", index)
    files = sorted(state.glob("kellu.log*"))
    assert len(files) >= 2
    assert all(_mode(path) == 0o600 for path in files)


def test_unknown_level_is_rejected(fake_home: Path) -> None:
    with pytest.raises(ConfigError):
        log.configure_logging(fake_home / "state", level="chatty")


def test_logger_does_not_propagate_to_root(fake_home: Path) -> None:
    logger = log.configure_logging(fake_home / "state")
    assert logger.propagate is False
