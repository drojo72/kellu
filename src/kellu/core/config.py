"""Configuration: packaged defaults merged with an optional user ``config.toml``.

Design rules (see SECURITY.md):

* Configuration is data, never code, and never holds secrets. Keys that look like secrets are
  rejected, with a pointer to the keyring.
* A tampered configuration could redirect API traffic, so the user file must be a regular file
  owned by the current user and not writable by group or others, endpoints must use HTTPS (plain
  HTTP only for localhost), and unknown keys are errors rather than silently ignored.
* Loading has no side effects: nothing is created, and there are no global singletons.
"""

from __future__ import annotations

import errno
import os
import re
import stat
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from importlib import resources
from pathlib import Path
from types import MappingProxyType
from typing import Any

from kellu.common import paths
from kellu.common.errors import ConfigError, SecurityError
from kellu.common.net import endpoint_problem
from kellu.providers.registry import KINDS as PROVIDER_KINDS
from kellu.providers.types import ProviderConfig

CONFIG_FILENAME = "config.toml"
MAX_CONFIG_BYTES = 256 * 1024
FALLBACK_ROUTE = "general"
LOG_LEVELS = frozenset({"DEBUG", "INFO", "WARNING", "ERROR"})

_NAME_RE = re.compile(r"^[a-z][a-z0-9_-]{0,31}$")
_SECRET_KEYS = frozenset(
    {"api_key", "apikey", "token", "secret", "password", "passwd", "authorization", "bearer"}
)
_SECRET_SUFFIXES = ("_api_key", "_apikey", "_token", "_secret", "_password")
_NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)
_CLOEXEC = getattr(os, "O_CLOEXEC", 0)

_TOP_KEYS = frozenset({"providers", "routing", "router", "logging", "sessions"})
_PROVIDER_KEYS = frozenset({"kind", "base_url", "model", "timeout_s", "keyring_name"})
_ROUTE_KEYS = frozenset({"primary", "backup"})
_ROUTER_KEYS = frozenset({"total_timeout_s"})
_LOGGING_KEYS = frozenset({"level", "max_bytes", "backups"})
_SESSIONS_KEYS = frozenset({"enabled", "max_entry_chars"})


@dataclass(frozen=True)
class Route:
    """Ordered provider chain for a task type."""

    primary: str
    backup: tuple[str, ...]

    @property
    def chain(self) -> tuple[str, ...]:
        """Providers to try, in order."""
        return (self.primary, *self.backup)


@dataclass(frozen=True)
class RouterConfig:
    """Limits for the provider router."""

    total_timeout_s: float


@dataclass(frozen=True)
class LoggingConfig:
    """Application log settings."""

    level: str
    max_bytes: int
    backups: int


@dataclass(frozen=True)
class SessionsConfig:
    """Session log settings."""

    enabled: bool
    max_entry_chars: int


@dataclass(frozen=True)
class Config:
    """Validated, immutable configuration."""

    providers: Mapping[str, ProviderConfig]
    routing: Mapping[str, Route]
    router: RouterConfig
    logging: LoggingConfig
    sessions: SessionsConfig
    source: Path | None = None  # the user file that was applied, if any

    def route(self, task: str) -> Route:
        """Return the route for ``task``, falling back to the ``general`` route."""
        return self.routing.get(task, self.routing[FALLBACK_ROUTE])


# --------------------------------------------------------------------- helpers ----
def _as_table(value: object, where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ConfigError(f"{where}: expected a table")
    return value


def _check_keys(table: Mapping[str, Any], allowed: frozenset[str], where: str) -> None:
    unknown = sorted(set(table) - allowed)
    if unknown:
        raise ConfigError(f"{where}: unknown key(s): {', '.join(unknown)}")


def _reject_secret_keys(node: object, where: str) -> None:
    if isinstance(node, dict):
        for key, value in node.items():
            low = str(key).lower()
            if low in _SECRET_KEYS or low.endswith(_SECRET_SUFFIXES):
                raise ConfigError(
                    f"{where}.{key}: secrets must not be stored in the configuration; "
                    "use `kellu keys set` to store them in the keyring"
                )
            _reject_secret_keys(value, f"{where}.{key}")
    elif isinstance(node, list):
        for index, item in enumerate(node):
            _reject_secret_keys(item, f"{where}[{index}]")


def _deep_merge(base: Mapping[str, Any], override: Mapping[str, Any]) -> dict[str, Any]:
    merged: dict[str, Any] = {}
    for key in base.keys() | override.keys():
        if key in base and key in override:
            left, right = base[key], override[key]
            if isinstance(left, dict) and isinstance(right, dict):
                merged[key] = _deep_merge(left, right)
            else:
                merged[key] = right
        else:
            merged[key] = override[key] if key in override else base[key]
    return merged


def _get_str(table: Mapping[str, Any], key: str, where: str) -> str:
    value = table.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ConfigError(f"{where}.{key}: expected a non-empty string")
    return value


def _get_int(table: Mapping[str, Any], key: str, where: str, low: int, high: int) -> int:
    value = table.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or not low <= value <= high:
        raise ConfigError(f"{where}.{key}: expected an integer between {low} and {high}")
    return value


def _get_name(value: object, where: str) -> str:
    if not isinstance(value, str) or not _NAME_RE.match(value):
        raise ConfigError(f"{where}: invalid name {value!r} (use a-z, 0-9, _ or -; max 32)")
    return value


def _validate_base_url(url: str, where: str) -> str:
    problem = endpoint_problem(url)
    if problem is not None:
        raise ConfigError(f"{where}.base_url: {problem}")
    return url.rstrip("/")


# --------------------------------------------------------------------- parsing ----
def _parse_provider(name: str, raw: object) -> ProviderConfig:
    where = f"providers.{_get_name(name, 'providers')}"
    table = _as_table(raw, where)
    _check_keys(table, _PROVIDER_KEYS, where)
    kind = _get_str(table, "kind", where)
    if kind not in PROVIDER_KINDS:
        raise ConfigError(f"{where}.kind: must be one of {', '.join(sorted(PROVIDER_KINDS))}")
    timeout = table.get("timeout_s")
    if isinstance(timeout, bool) or not isinstance(timeout, int | float) or not 1 <= timeout <= 300:
        raise ConfigError(f"{where}.timeout_s: expected a number between 1 and 300")
    keyring_name = table.get("keyring_name")
    if keyring_name is not None:
        keyring_name = _get_name(keyring_name, f"{where}.keyring_name")
    return ProviderConfig(
        name=name,
        kind=kind,
        base_url=_validate_base_url(_get_str(table, "base_url", where), where),
        model=_get_str(table, "model", where),
        timeout_s=float(timeout),
        keyring_name=keyring_name,
    )


def _parse_route(name: str, raw: object, providers: Mapping[str, ProviderConfig]) -> Route:
    where = f"routing.{_get_name(name, 'routing')}"
    table = _as_table(raw, where)
    _check_keys(table, _ROUTE_KEYS, where)
    primary = _get_str(table, "primary", where)
    backup_raw = table.get("backup", [])
    if not isinstance(backup_raw, list) or not all(isinstance(item, str) for item in backup_raw):
        raise ConfigError(f"{where}.backup: expected a list of provider names")
    chain = [primary, *backup_raw]
    unknown = [item for item in chain if item not in providers]
    if unknown:
        raise ConfigError(f"{where}: unknown provider(s): {', '.join(unknown)}")
    if len(set(chain)) != len(chain):
        raise ConfigError(f"{where}: a provider appears more than once in the chain")
    return Route(primary=primary, backup=tuple(backup_raw))


def parse_config(
    defaults: Mapping[str, Any],
    user: Mapping[str, Any] | None = None,
    *,
    source: Path | None = None,
) -> Config:
    """Merge ``user`` over ``defaults`` and validate the result. Pure: no filesystem access."""
    merged = _deep_merge(defaults, user or {})
    _reject_secret_keys(merged, "config")
    _check_keys(merged, _TOP_KEYS, "config")

    providers_raw = _as_table(merged.get("providers", {}), "providers")
    providers = {name: _parse_provider(name, raw) for name, raw in providers_raw.items()}
    if not providers:
        raise ConfigError("providers: at least one provider is required")

    routing_raw = _as_table(merged.get("routing", {}), "routing")
    routing = {name: _parse_route(name, raw, providers) for name, raw in routing_raw.items()}
    if FALLBACK_ROUTE not in routing:
        raise ConfigError(f"routing.{FALLBACK_ROUTE}: the fallback route is required")

    router_table = _as_table(merged.get("router", {}), "router")
    _check_keys(router_table, _ROUTER_KEYS, "router")
    total = router_table.get("total_timeout_s")
    if isinstance(total, bool) or not isinstance(total, int | float) or not 5 <= total <= 900:
        raise ConfigError("router.total_timeout_s: expected a number between 5 and 900")
    router_cfg = RouterConfig(total_timeout_s=float(total))

    log_table = _as_table(merged.get("logging", {}), "logging")
    _check_keys(log_table, _LOGGING_KEYS, "logging")
    level = _get_str(log_table, "level", "logging").upper()
    if level not in LOG_LEVELS:
        raise ConfigError(f"logging.level: must be one of {', '.join(sorted(LOG_LEVELS))}")
    logging_cfg = LoggingConfig(
        level=level,
        max_bytes=_get_int(log_table, "max_bytes", "logging", 10_000, 100_000_000),
        backups=_get_int(log_table, "backups", "logging", 0, 20),
    )

    sess_table = _as_table(merged.get("sessions", {}), "sessions")
    _check_keys(sess_table, _SESSIONS_KEYS, "sessions")
    enabled = sess_table.get("enabled")
    if not isinstance(enabled, bool):
        raise ConfigError("sessions.enabled: expected true or false")
    sessions_cfg = SessionsConfig(
        enabled=enabled,
        max_entry_chars=_get_int(sess_table, "max_entry_chars", "sessions", 100, 100_000),
    )

    return Config(
        providers=MappingProxyType(providers),
        routing=MappingProxyType(routing),
        router=router_cfg,
        logging=logging_cfg,
        sessions=sessions_cfg,
        source=source,
    )


# --------------------------------------------------------------------- loading ----
def user_config_path() -> Path:
    """Location of the user configuration file (it may not exist)."""
    return paths.config_dir() / CONFIG_FILENAME


def load_defaults() -> dict[str, Any]:
    """Read the packaged defaults."""
    text = resources.files("kellu.core").joinpath("defaults.toml").read_text(encoding="utf-8")
    return tomllib.loads(text)


def _read_user_file(path: Path) -> dict[str, Any] | None:
    """Read and safety-check the user file. Returns ``None`` when it does not exist."""
    try:
        fd = os.open(path, os.O_RDONLY | _NOFOLLOW | _CLOEXEC)
    except FileNotFoundError:
        return None
    except OSError as exc:
        if exc.errno == errno.ELOOP:
            raise SecurityError(f"{path} is a symlink; refusing to follow it") from exc
        raise ConfigError(f"{path}: cannot be read ({exc.strerror})") from exc
    with os.fdopen(fd, "rb") as handle:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise SecurityError(f"{path} must be a regular file")
        if hasattr(os, "getuid") and info.st_uid != os.getuid():
            raise SecurityError(f"{path} is owned by another user")
        if info.st_mode & 0o022:
            raise SecurityError(f"{path} is writable by group or others; run: chmod go-w {path}")
        raw = handle.read(MAX_CONFIG_BYTES + 1)
    if len(raw) > MAX_CONFIG_BYTES:
        raise ConfigError(f"{path}: larger than {MAX_CONFIG_BYTES} bytes")
    try:
        return tomllib.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        raise ConfigError(f"{path}: invalid TOML ({exc})") from exc


def load_config(user_file: Path | None = None) -> Config:
    """Load packaged defaults plus the user file (default: ``~/.config/kellu/config.toml``).

    Raises:
        ConfigError: the content is invalid.
        SecurityError: the user file is unsafe (symlink, wrong owner, group/world-writable).
    """
    path = user_file if user_file is not None else user_config_path()
    user = _read_user_file(path)
    return parse_config(load_defaults(), user, source=path if user is not None else None)
