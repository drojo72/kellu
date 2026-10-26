from __future__ import annotations

import logging
from collections.abc import Iterator

import pytest

from kellu.common.errors import ProviderError
from kellu.core import config as cfg
from kellu.core.router import AllProvidersFailedError, Router, classify_task
from kellu.providers.registry import build_provider
from kellu.providers.types import (
    ChatRequest,
    ChatResponse,
    Message,
    ProviderConfig,
    StreamEvent,
    ToolSpec,
)

REQUEST = ChatRequest(messages=(Message("user", "hi"),))
TOOL_REQUEST = ChatRequest(
    messages=(Message("user", "hi"),), tools=(ToolSpec("t", "d", {"type": "object"}),)
)


class FakeProvider:
    """Scripted provider: each call consumes the next outcome (a text or an exception)."""

    def __init__(self, name: str, *outcomes: str | Exception, supports_tools: bool = True) -> None:
        self.name = name
        self.supports_tools = supports_tools
        self.calls = 0
        self._outcomes = list(outcomes)

    def _next(self) -> str | Exception:
        self.calls += 1
        return self._outcomes.pop(0)

    def chat(self, request: ChatRequest) -> ChatResponse:
        outcome = self._next()
        if isinstance(outcome, Exception):
            raise outcome
        return ChatResponse(text=outcome, provider=self.name)

    def stream(self, request: ChatRequest) -> Iterator[StreamEvent]:
        outcome = self._next()
        if isinstance(outcome, Exception):
            raise outcome
        for piece in outcome.split("|"):
            yield StreamEvent(kind="text", text=piece)
        yield StreamEvent(
            kind="done", response=ChatResponse(text=outcome.replace("|", ""), provider=self.name)
        )


class FailAfterFirstChunk(FakeProvider):
    def stream(self, request: ChatRequest) -> Iterator[StreamEvent]:
        self.calls += 1
        yield StreamEvent(kind="text", text="partial")
        raise ProviderError(self.name, "connection lost")


def _error(
    name: str, message: str = "boom", *, status: int = 500, retryable: bool = True
) -> ProviderError:
    return ProviderError(name, message, status=status, retryable=retryable)


def _router(providers: dict[str, FakeProvider], clock=None) -> Router:  # type: ignore[no-untyped-def]
    config = cfg.parse_config(cfg.load_defaults())
    kwargs = {"clock": clock} if clock else {}
    return Router(config, providers, **kwargs)  # type: ignore[arg-type]


def _all(**overrides: FakeProvider) -> dict[str, FakeProvider]:
    base = {
        n: FakeProvider(n, f"{n}-answer")
        for n in ("gemma_direct", "google", "deepseek", "openrouter", "grok")
    }
    base.update(overrides)
    return base


# ------------------------------------------------------------------ classification ----
@pytest.mark.parametrize(
    ("prompt", "expected"),
    [
        ("```python\nprint(1)\n```", "code"),
        ("def foo():\n    pass", "code"),
        ("Hay un bug en mi función de login", "code"),
        ("tengo un traceback al compilar", "code"),
        ("write a python script to rename files", "code"),
        ("Explica cómo funciona un hash", "explain"),
        ("¿Qué es un mutex?", "general"),
        ("Qué es un mutex", "explain"),
        ("Explain what a mutex is", "explain"),
        ("how does TLS work", "explain"),
        ("hola, cómo estás", "general"),
        ("Dame ideas para el nombre de mi tienda", "general"),
    ],
)
def test_classify_task(prompt: str, expected: str) -> None:
    assert classify_task(prompt) == expected


def test_spanish_word_como_does_not_change_the_task_language() -> None:
    # Regression: the prototype treated any prompt with "como" as Portuguese.
    assert classify_task("como puedo ordenar una lista") == "general"


# ----------------------------------------------------------------------- fallback ----
def test_first_provider_answers_when_it_works() -> None:
    providers = _all()
    response = _router(providers).chat("general", REQUEST)
    assert response.provider == "gemma_direct"
    assert providers["google"].calls == 0


def test_retryable_error_falls_back_to_the_next_provider() -> None:
    providers = _all(gemma_direct=FakeProvider("gemma_direct", _error("gemma_direct", status=429)))
    assert _router(providers).chat("general", REQUEST).provider == "google"


def test_non_retryable_error_also_falls_back() -> None:
    bad_key = _error("gemma_direct", "no API key", status=0, retryable=False)
    providers = _all(gemma_direct=FakeProvider("gemma_direct", bad_key))
    assert _router(providers).chat("general", REQUEST).provider == "google"


def test_task_selects_its_own_chain() -> None:
    providers = _all()
    assert _router(providers).chat("code", REQUEST).provider == "deepseek"
    assert _router(providers).chat("explain", REQUEST).provider == "google"
    assert _router(providers).chat("unknown-task", REQUEST).provider == "gemma_direct"


def test_all_failures_are_reported_in_order() -> None:
    providers = _all(
        deepseek=FakeProvider("deepseek", _error("deepseek", "overloaded", status=503)),
        openrouter=FakeProvider(
            "openrouter", _error("openrouter", "bad request", status=400, retryable=False)
        ),
    )
    with pytest.raises(AllProvidersFailedError) as exc:
        _router(providers).chat("code", REQUEST)
    failures = exc.value.failures
    assert [f.provider for f in failures] == ["deepseek", "openrouter"]
    assert failures[0].retryable is True and failures[1].retryable is False
    assert "deepseek: overloaded" in str(exc.value)


def test_missing_provider_is_recorded_and_skipped() -> None:
    providers = _all()
    del providers["deepseek"]
    assert _router(providers).chat("code", REQUEST).provider == "openrouter"


def test_providers_without_tool_support_are_skipped_when_tools_are_requested() -> None:
    providers = _all(
        gemma_direct=FakeProvider("gemma_direct", "x", supports_tools=False),
        google=FakeProvider("google", "x", supports_tools=False),
    )
    response = _router(providers).chat("general", TOOL_REQUEST)
    assert response.provider == "deepseek"
    assert providers["gemma_direct"].calls == 0


def test_nothing_is_tried_when_no_provider_supports_the_request() -> None:
    providers = {n: FakeProvider(n, "x", supports_tools=False) for n in _all()}
    with pytest.raises(AllProvidersFailedError) as exc:
        _router(providers).chat("code", TOOL_REQUEST)
    assert all("tool calling" in f.message for f in exc.value.failures)


def test_total_time_budget_stops_new_attempts() -> None:
    now = [0.0]

    class SlowFailure(FakeProvider):
        def chat(self, request: ChatRequest) -> ChatResponse:
            self.calls += 1
            now[0] += 200.0  # more than router.total_timeout_s (120), spent inside the call
            raise _error(self.name, "slow")

    providers = _all(gemma_direct=SlowFailure("gemma_direct"))
    with pytest.raises(AllProvidersFailedError) as exc:
        _router(providers, clock=lambda: now[0]).chat("general", REQUEST)
    assert providers["google"].calls == 0
    assert exc.value.failures[-1].message == "total time budget exhausted"


def test_failures_are_logged_without_secrets(caplog: pytest.LogCaptureFixture) -> None:
    providers = _all(
        gemma_direct=FakeProvider("gemma_direct", _error("gemma_direct", "quota", status=429))
    )
    with caplog.at_level(logging.WARNING, logger="kellu.core.router"):
        _router(providers).chat("general", REQUEST)
    assert "gemma_direct failed" in caplog.text and "quota" in caplog.text


# --------------------------------------------------------------------- streaming ----
def test_stream_returns_events_from_the_first_provider() -> None:
    providers = _all(gemma_direct=FakeProvider("gemma_direct", "Hel|lo"))
    events = list(_router(providers).stream("general", REQUEST))
    assert [e.text for e in events if e.kind == "text"] == ["Hel", "lo"]
    assert events[-1].kind == "done"


def test_stream_fails_over_before_any_text_was_delivered() -> None:
    providers = _all(gemma_direct=FakeProvider("gemma_direct", _error("gemma_direct", status=503)))
    events = list(_router(providers).stream("general", REQUEST))
    assert events[-1].response is not None and events[-1].response.provider == "google"


def test_stream_does_not_fail_over_after_text_was_delivered() -> None:
    providers = _all(gemma_direct=FailAfterFirstChunk("gemma_direct"))
    iterator = _router(providers).stream("general", REQUEST)
    assert next(iterator).text == "partial"
    with pytest.raises(ProviderError, match="connection lost"):
        next(iterator)
    assert providers["google"].calls == 0


def test_stream_that_never_finishes_counts_as_a_failure() -> None:
    class NeverDone(FakeProvider):
        def stream(self, request: ChatRequest) -> Iterator[StreamEvent]:
            self.calls += 1
            return iter(())

    providers = _all(gemma_direct=NeverDone("gemma_direct"))
    events = list(_router(providers).stream("general", REQUEST))
    assert events[-1].response is not None and events[-1].response.provider == "google"


def test_stream_reports_all_failures() -> None:
    providers = {n: FakeProvider(n, _error(n)) for n in _all()}
    with pytest.raises(AllProvidersFailedError):
        list(_router(providers).stream("code", REQUEST))


# ------------------------------------------------------------------------ wiring ----
def test_from_config_builds_one_provider_per_configured_entry() -> None:
    config = cfg.parse_config(cfg.load_defaults())
    router = Router.from_config(config, lambda _name: None)
    assert set(router._providers) == set(config.providers)  # noqa: SLF001


def test_build_provider_rejects_unknown_kind() -> None:
    from kellu.common.errors import ConfigError

    bad = ProviderConfig("x", "carrier-pigeon", "https://example.com", "m", 5.0, None)
    with pytest.raises(ConfigError, match="unknown kind"):
        build_provider(bad, lambda _n: None)
