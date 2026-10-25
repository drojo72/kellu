"""XDG base-directory resolution and private file and directory creation.

Resolving a path never touches the filesystem. Directories and files are created only by the
``ensure_*`` and ``open_*`` helpers, and only when a caller explicitly asks for it.
"""

from __future__ import annotations

import errno
import os
import stat
from io import TextIOWrapper
from pathlib import Path

from kellu.__app__ import APP_NAME
from kellu.common.errors import SecurityError

PRIVATE_DIR_MODE = 0o700
PRIVATE_FILE_MODE = 0o600

_NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)
_CLOEXEC = getattr(os, "O_CLOEXEC", 0)


def _xdg_base(env_var: str, default_relative: str) -> Path:
    """Return an XDG base directory.

    The XDG specification says relative values must be ignored.
    """
    raw = os.environ.get(env_var, "")
    if raw and os.path.isabs(raw):
        return Path(raw)
    return Path.home() / default_relative


def config_dir() -> Path:
    """Providers, routing, scopes, sandbox policy."""
    return _xdg_base("XDG_CONFIG_HOME", ".config") / APP_NAME


def state_dir() -> Path:
    """Session logs and history."""
    return _xdg_base("XDG_STATE_HOME", ".local/state") / APP_NAME


def cache_dir() -> Path:
    """Disposable caches."""
    return _xdg_base("XDG_CACHE_HOME", ".cache") / APP_NAME


def ensure_private_dir(path: Path) -> Path:
    """Create ``path`` with mode 0700 if needed and verify it is safe to use.

    Raises:
        SecurityError: ``path`` is a symlink or not a directory, or is owned by another user.
        OSError: the directory cannot be created.
    """
    path.mkdir(parents=True, exist_ok=True, mode=PRIVATE_DIR_MODE)
    info = path.lstat()
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
        raise SecurityError(f"{path} must be a real directory, not a symlink or a file")
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        raise SecurityError(f"{path} is owned by another user")
    if stat.S_IMODE(info.st_mode) != PRIVATE_DIR_MODE:
        path.chmod(PRIVATE_DIR_MODE)
    return path


def open_private_append(path: Path, *, new_only: bool = False) -> TextIOWrapper:
    """Open ``path`` for appending text (UTF-8), creating it with mode 0600.

    The file is opened without following symlinks and verified on the open descriptor
    (regular file, owned by the current user), which avoids check-then-open races.

    Args:
        new_only: fail with ``FileExistsError`` if the file already exists.

    Raises:
        SecurityError: ``path`` is a symlink, not a regular file, or owned by another user.
    """
    flags = os.O_WRONLY | os.O_APPEND | os.O_CREAT | _NOFOLLOW | _CLOEXEC
    if new_only:
        flags |= os.O_EXCL
    try:
        fd = os.open(path, flags, PRIVATE_FILE_MODE)
    except OSError as exc:
        if exc.errno == errno.ELOOP:
            raise SecurityError(f"{path} is a symlink; refusing to follow it") from exc
        raise
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode):
            raise SecurityError(f"{path} is not a regular file")
        if hasattr(os, "getuid") and info.st_uid != os.getuid():
            raise SecurityError(f"{path} is owned by another user")
        if stat.S_IMODE(info.st_mode) != PRIVATE_FILE_MODE:
            os.fchmod(fd, PRIVATE_FILE_MODE)
        return os.fdopen(fd, "a", encoding="utf-8")
    except BaseException:
        os.close(fd)
        raise


def ensure_private_file(path: Path) -> Path:
    """Create ``path`` with mode 0600 if needed (tightening permissions) and verify it."""
    open_private_append(path).close()
    return path
