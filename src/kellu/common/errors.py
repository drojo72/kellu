"""Exception hierarchy."""

from __future__ import annotations


class KelluError(Exception):
    """Base class for all Kellu errors."""


class ConfigError(KelluError):
    """Invalid or unreadable configuration."""


class SecurityError(KelluError):
    """A security invariant would be violated (unsafe path, wrong ownership, denied access)."""


class SessionError(KelluError):
    """A session log was used incorrectly (not started, already ended, bad entry kind)."""


class ProviderError(KelluError):
    """A model provider call failed.

    ``retryable`` is a hint for the router: transient failures (timeouts, 429, 5xx) are
    retryable, configuration problems (missing key, bad request) are not. Messages never contain
    secrets.
    """

    def __init__(
        self, provider: str, message: str, *, status: int = 0, retryable: bool = True
    ) -> None:
        suffix = f" (HTTP {status})" if status else ""
        super().__init__(f"[{provider}] {message}{suffix}")
        self.provider = provider
        self.status = status
        self.retryable = retryable
        self.reason = message
