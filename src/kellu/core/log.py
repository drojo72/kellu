"""Logging setup: private files, rotation that keeps mode 0600, and redaction.

Nothing here runs at import time. Call :func:`configure_logging` explicitly.
"""

from __future__ import annotations

import logging
import logging.handlers
from io import TextIOWrapper
from pathlib import Path

from kellu.__app__ import APP_NAME
from kellu.common import paths
from kellu.common.errors import ConfigError
from kellu.common.text import Redactor, identity_redactor, strip_control_chars

LOG_FILENAME = "kellu.log"
LEVELS = frozenset({"DEBUG", "INFO", "WARNING", "ERROR"})
_HANDLER_MARK = "_kellu_handler"
_FORMAT = "%(asctime)s %(levelname)s %(name)s: %(message)s"


class RedactingFormatter(logging.Formatter):
    """Format a record, then redact secrets and strip control characters from the result.

    Redacting the final string also covers exception text and formatted arguments.
    """

    def __init__(self, redact: Redactor) -> None:
        super().__init__(_FORMAT)
        self._redact = redact

    def format(self, record: logging.LogRecord) -> str:
        return strip_control_chars(self._redact(super().format(record)))


class _PrivateRotatingFileHandler(logging.handlers.RotatingFileHandler):
    """Rotating handler whose files are always opened privately (0600, no symlinks)."""

    def _open(self) -> TextIOWrapper:
        return paths.open_private_append(Path(self.baseFilename))


def configure_logging(
    state_dir: Path,
    *,
    level: str = "INFO",
    max_bytes: int = 1_000_000,
    backups: int = 3,
    redact: Redactor = identity_redactor,
) -> logging.Logger:
    """Attach a private rotating file handler to the ``kellu`` logger and return it.

    Idempotent: calling it again replaces the handler instead of adding another one.
    """
    level_name = level.upper()
    if level_name not in LEVELS:
        raise ConfigError(f"unknown log level: {level}")
    paths.ensure_private_dir(state_dir)

    logger = logging.getLogger(APP_NAME)
    for existing in list(logger.handlers):
        if getattr(existing, _HANDLER_MARK, False):
            logger.removeHandler(existing)
            existing.close()

    handler = _PrivateRotatingFileHandler(
        state_dir / LOG_FILENAME, maxBytes=max_bytes, backupCount=backups, encoding="utf-8"
    )
    setattr(handler, _HANDLER_MARK, True)
    handler.setFormatter(RedactingFormatter(redact))
    logger.addHandler(handler)
    logger.setLevel(level_name)
    logger.propagate = False
    return logger
