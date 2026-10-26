"""Task classification and provider routing with fallback.

The router walks the configured chain for a task type and moves to the next provider when one
fails. Rules worth knowing:

* A provider that cannot do what the request needs (tools) is skipped, not tried.
* No new attempt starts after the total time budget is spent.
* A stream can fail over only **before** any text was delivered. After that, switching providers
  would duplicate or contradict what the user already saw, so the error is raised instead.
* Failure messages come from the providers and never contain secrets.
"""

from __future__ import annotations

import logging
import re
import time
import unicodedata
from collections.abc import Callable, Iterator, Mapping
from dataclasses import dataclass

from kellu.common.errors import KelluError, ProviderError
from kellu.core.config import Config
from kellu.providers.base import KeyLookup, Provider
from kellu.providers.registry import build_provider
from kellu.providers.types import ChatRequest, ChatResponse, StreamEvent

_log = logging.getLogger(__name__)

Clock = Callable[[], float]

_EXPLAIN_PREFIXES = (
    "explain",
    "explica",
    "what is",
    "what are",
    "how does",
    "how do",
    "why ",
    "que es",
    "que son",
    "como funciona",
    "por que",
)
_CODE_WORDS = frozenset(
    {
        "bug", "traceback", "exception", "stacktrace", "refactor", "refactoriza", "compile",
        "compilar", "function", "funcion", "script", "regex", "sql", "python", "javascript",
        "rust", "bash", "git", "codigo", "code", "clase", "class", "typescript", "docker",
    }
)  # fmt: skip
_CODE_LINE_RE = re.compile(
    r"^\s*(def|class|import|from|const|let|fn|func|#include)\b", re.MULTILINE
)
_WORD_RE = re.compile(r"[a-z0-9_#+]+")


def _fold(text: str) -> str:
    """Lowercase and strip accents so Spanish and English match the same word lists."""
    decomposed = unicodedata.normalize("NFKD", text.lower())
    return "".join(ch for ch in decomposed if not unicodedata.combining(ch))


def classify_task(prompt: str) -> str:
    """Return ``code``, ``explain`` or ``general`` for a user prompt (Spanish or English)."""
    folded = _fold(prompt)
    if "```" in folded or _CODE_LINE_RE.search(folded):
        return "code"
    if folded.lstrip().startswith(_EXPLAIN_PREFIXES):
        return "explain"
    if _CODE_WORDS.intersection(_WORD_RE.findall(folded)):
        return "code"
    return "general"


@dataclass(frozen=True)
class ProviderFailure:
    """Why one provider in the chain did not produce an answer."""

    provider: str
    status: int
    message: str
    retryable: bool


class AllProvidersFailedError(KelluError):
    """Every provider in the chain failed or was skipped."""

    def __init__(self, task: str, failures: tuple[ProviderFailure, ...]) -> None:
        detail = "; ".join(f"{f.provider}: {f.message}" for f in failures) or "empty chain"
        super().__init__(f"all providers failed for task '{task}': {detail}")
        self.task = task
        self.failures = failures


class Router:
    """Routes chat requests to providers according to the configuration."""

    def __init__(
        self,
        config: Config,
        providers: Mapping[str, Provider],
        *,
        clock: Clock = time.monotonic,
    ) -> None:
        self._config = config
        self._providers = providers
        self._clock = clock

    @classmethod
    def from_config(
        cls, config: Config, key_lookup: KeyLookup, *, clock: Clock = time.monotonic
    ) -> Router:
        """Build every configured provider. Keys are looked up per request, not here."""
        providers = {
            name: build_provider(provider_config, key_lookup)
            for name, provider_config in config.providers.items()
        }
        return cls(config, providers, clock=clock)

    # ------------------------------------------------------------------- chat ----
    def chat(self, task: str, request: ChatRequest) -> ChatResponse:
        """Return the first successful answer along the chain for ``task``."""
        failures: list[ProviderFailure] = []
        for name in self._attempts(task, request, failures):
            try:
                return self._providers[name].chat(request)
            except ProviderError as exc:
                failures.append(self._record(name, exc))
        raise AllProvidersFailedError(task, tuple(failures))

    def stream(self, task: str, request: ChatRequest) -> Iterator[StreamEvent]:
        """Stream the answer. Fails over only until the first text has been delivered."""
        failures: list[ProviderFailure] = []
        for name in self._attempts(task, request, failures):
            delivered = False
            finished = False
            try:
                for event in self._providers[name].stream(request):
                    if event.kind == "text" and event.text:
                        delivered = True
                    finished = finished or event.kind == "done"
                    yield event
                if not finished:
                    raise ProviderError(name, "stream ended without a final event")
                return
            except ProviderError as exc:
                if delivered:
                    raise
                failures.append(self._record(name, exc))
        raise AllProvidersFailedError(task, tuple(failures))

    # ---------------------------------------------------------------- internals ----
    def _attempts(
        self, task: str, request: ChatRequest, failures: list[ProviderFailure]
    ) -> Iterator[str]:
        """Yield provider names worth trying, recording the ones that are skipped."""
        deadline = self._clock() + self._config.router.total_timeout_s
        for name in self._config.route(task).chain:
            provider = self._providers.get(name)
            if provider is None:
                failures.append(ProviderFailure(name, 0, "provider is not available", False))
            elif request.tools and not provider.supports_tools:
                failures.append(ProviderFailure(name, 0, "does not support tool calling", False))
            elif self._clock() >= deadline:
                failures.append(ProviderFailure(name, 0, "total time budget exhausted", False))
                return
            else:
                yield name

    @staticmethod
    def _record(name: str, exc: ProviderError) -> ProviderFailure:
        _log.warning(
            "provider %s failed (status %s, retryable=%s): %s",
            name,
            exc.status,
            exc.retryable,
            exc.reason,
        )
        return ProviderFailure(name, exc.status, exc.reason, exc.retryable)
