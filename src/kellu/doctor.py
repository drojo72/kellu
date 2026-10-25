"""Environment checks for ``kellu doctor``.

Basic so far: tool presence, configuration validity and paths. Sandbox probing and keyring
access checks arrive in M4.
"""

from __future__ import annotations

import shutil
import sys
from collections.abc import Callable
from dataclasses import dataclass

from kellu.common import paths
from kellu.common.errors import KelluError
from kellu.core.config import Config, load_config

MIN_PYTHON = (3, 11)

Which = Callable[[str], str | None]
ConfigLoader = Callable[[], Config]


@dataclass(frozen=True)
class Check:
    """Result of one environment check."""

    name: str
    ok: bool
    required: bool
    detail: str


def _tool_check(which: Which, binary: str, *, required: bool, purpose: str, hint: str) -> Check:
    found = which(binary)
    detail = found if found else f"not found; {purpose}. {hint}"
    return Check(binary, found is not None, required, detail)


def _config_check(load: ConfigLoader) -> Check:
    try:
        config = load()
    except KelluError as exc:
        return Check("config", False, True, str(exc))
    where = str(config.source) if config.source else "packaged defaults"
    return Check("config", True, True, f"{where} ({len(config.providers)} providers)")


def collect_checks(which: Which = shutil.which, load: ConfigLoader = load_config) -> list[Check]:
    """Run all checks. ``which`` and ``load`` are injectable so tests never depend on the host."""
    py_ok = sys.version_info[:2] >= MIN_PYTHON
    return [
        Check(
            "python",
            py_ok,
            True,
            f"{sys.version_info.major}.{sys.version_info.minor} "
            f"(need >= {MIN_PYTHON[0]}.{MIN_PYTHON[1]})",
        ),
        _config_check(load),
        _tool_check(
            which,
            "secret-tool",
            required=True,
            purpose="API keys cannot be stored in the keyring",
            hint="Debian/Ubuntu: sudo apt install libsecret-tools",
        ),
        _tool_check(
            which,
            "bwrap",
            required=False,
            purpose="the shell tool stays disabled (fail closed)",
            hint="Debian/Ubuntu: sudo apt install bubblewrap",
        ),
        _tool_check(
            which,
            "git",
            required=False,
            purpose="git tools are unavailable",
            hint="Debian/Ubuntu: sudo apt install git",
        ),
    ]


def run(
    which: Which = shutil.which,
    out: Callable[[str], None] = print,
    load: ConfigLoader = load_config,
) -> int:
    """Print a report and return the process exit code (0 when all required checks pass)."""
    checks = collect_checks(which, load)
    for check in checks:
        status = "ok" if check.ok else ("FAIL" if check.required else "warn")
        out(f"[{status:>4}] {check.name}: {check.detail}")
    out("")
    out(f"config: {paths.config_dir()}")
    out(f"state:  {paths.state_dir()}")
    out(f"cache:  {paths.cache_dir()}")
    out("")
    out("Sandbox probing (user namespaces, bubblewrap policy) arrives in milestone M4.")
    return 0 if all(c.ok for c in checks if c.required) else 1
