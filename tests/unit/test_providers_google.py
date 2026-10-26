from __future__ import annotations

import json

import pytest

from conftest import LLMServer, Reply
from kellu.common.errors import ProviderError
from kellu.providers.google import GoogleProvider
from kellu.providers.types import ChatRequest, Message, ProviderConfig, ToolSpec

KEY = "AIza-test-0123456789"


def _provider(
    server: LLMServer, model: str = "gemini-flash-latest", key: str | None = KEY
) -> GoogleProvider:
    config = ProviderConfig(
        name="google",
        kind="google",
        base_url=server.base_url,
        model=model,
        timeout_s=5,
        keyring_name="google",
    )
    return GoogleProvider(config, lambda _name: key)


def _chat(*messages: Message, **kwargs: object) -> ChatRequest:
    return ChatRequest(messages=messages, **kwargs)  # type: ignore[arg-type]


def _answer(text: str, **extra: object) -> bytes:
    body = {
        "candidates": [
            {"content": {"role": "model", "parts": [{"text": text}]}, "finishReason": "STOP"}
        ],
        "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 2},
        "modelVersion": "gemini-x-001",
    }
    body.update(extra)
    return json.dumps(body).encode()


def _sse(*chunks: object) -> list[str]:
    return [f"data: {json.dumps(c)}\n\n" for c in chunks]


def test_chat_uses_header_auth_and_never_puts_the_key_in_the_url(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_answer("hola")))
    response = _provider(llm_server).chat(_chat(Message("user", "hi")))
    sent = llm_server.requests[0]
    assert sent.path == "/v1/models/gemini-flash-latest:generateContent"
    assert sent.headers["x-goog-api-key"] == KEY
    assert KEY not in sent.path and "key=" not in sent.path
    assert response.text == "hola"
    assert response.model == "gemini-x-001"
    assert response.finish_reason == "STOP"
    assert response.usage is not None and response.usage.prompt_tokens == 5


def test_gemini_uses_a_system_instruction(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_answer("ok")))
    _provider(llm_server).chat(_chat(Message("system", "be brief"), Message("user", "hi")))
    payload = json.loads(llm_server.requests[0].body)
    assert payload["systemInstruction"] == {"parts": [{"text": "be brief"}]}
    assert payload["contents"] == [{"role": "user", "parts": [{"text": "hi"}]}]


def test_gemma_folds_the_system_message_into_the_first_user_turn(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_answer("ok")))
    _provider(llm_server, model="gemma-4-26b-a4b-it").chat(
        _chat(Message("system", "be brief"), Message("user", "hi"))
    )
    payload = json.loads(llm_server.requests[0].body)
    assert "systemInstruction" not in payload
    assert payload["contents"][0]["role"] == "user"
    assert payload["contents"][0]["parts"][0]["text"] == "be brief\n\nhi"


def test_roles_are_mapped_and_generation_config_is_sent(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_answer("ok")))
    _provider(llm_server).chat(
        _chat(
            Message("user", "a"),
            Message("assistant", "b"),
            Message("user", "c"),
            temperature=0.1,
            max_tokens=64,
        )
    )
    payload = json.loads(llm_server.requests[0].body)
    assert [c["role"] for c in payload["contents"]] == ["user", "model", "user"]
    assert payload["generationConfig"] == {"temperature": 0.1, "maxOutputTokens": 64}


def test_thinking_parts_are_not_part_of_the_answer(llm_server: LLMServer) -> None:
    parts = [{"text": "secret reasoning", "thought": True}, {"text": "final"}]
    body = {"candidates": [{"content": {"parts": parts}, "finishReason": "STOP"}]}
    llm_server.queue(Reply(body=json.dumps(body).encode()))
    assert _provider(llm_server).chat(_chat(Message("user", "q"))).text == "final"


def test_blocked_prompt_is_a_non_retryable_error(llm_server: LLMServer) -> None:
    body = {"promptFeedback": {"blockReason": "SAFETY"}}
    llm_server.queue(Reply(body=json.dumps(body).encode()))
    with pytest.raises(ProviderError, match="blocked") as exc:
        _provider(llm_server).chat(_chat(Message("user", "q")))
    assert exc.value.retryable is False


def test_tools_are_not_supported_yet(llm_server: LLMServer) -> None:
    tool = ToolSpec("t", "d", {"type": "object"})
    provider = _provider(llm_server)
    assert provider.supports_tools is False
    with pytest.raises(ProviderError, match="not supported"):
        provider.chat(_chat(Message("user", "q"), tools=(tool,)))
    assert llm_server.requests == []


def test_missing_key_fails_before_the_network(llm_server: LLMServer) -> None:
    with pytest.raises(ProviderError, match="kellu keys set google"):
        _provider(llm_server, key=None).chat(_chat(Message("user", "q")))
    assert llm_server.requests == []


def test_http_error_and_key_scrubbing(llm_server: LLMServer) -> None:
    body = json.dumps({"error": {"code": 400, "message": f"API key not valid: {KEY}"}}).encode()
    llm_server.queue(Reply(status=400, body=body))
    with pytest.raises(ProviderError) as exc:
        _provider(llm_server).chat(_chat(Message("user", "q")))
    assert exc.value.retryable is False
    assert KEY not in str(exc.value)


def test_stream_uses_sse_endpoint_and_assembles_the_answer(llm_server: LLMServer) -> None:
    chunk = lambda text, **kw: {"candidates": [{"content": {"parts": [{"text": text}]}, **kw}]}  # noqa: E731
    llm_server.queue(
        Reply(
            sse=_sse(
                chunk("Hel"),
                chunk("lo", finishReason="STOP"),
                {
                    "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 2},
                    "modelVersion": "g-1",
                },
            )
        )
    )
    events = list(_provider(llm_server).stream(_chat(Message("user", "q"))))
    assert (
        llm_server.requests[0].path
        == "/v1/models/gemini-flash-latest:streamGenerateContent?alt=sse"
    )
    assert [e.text for e in events if e.kind == "text"] == ["Hel", "lo"]
    final = events[-1].response
    assert final is not None and final.text == "Hello"
    assert final.finish_reason == "STOP" and final.model == "g-1"
    assert final.usage is not None and final.usage.completion_tokens == 2


def test_stream_error_event_raises(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(sse=_sse({"error": {"code": 503, "message": "overloaded"}})))
    with pytest.raises(ProviderError, match="overloaded") as exc:
        list(_provider(llm_server).stream(_chat(Message("user", "q"))))
    assert exc.value.retryable is True


def test_empty_stream_is_an_error(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(sse=[]))
    with pytest.raises(ProviderError, match="without data"):
        list(_provider(llm_server).stream(_chat(Message("user", "q"))))
