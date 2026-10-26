from __future__ import annotations

import json

import pytest

from conftest import LLMServer, Reply
from kellu.common.errors import ProviderError
from kellu.providers import http
from kellu.providers.openai_compat import OpenAICompatProvider
from kellu.providers.types import (
    ChatRequest,
    Message,
    ProviderConfig,
    ToolCall,
    ToolSpec,
)

KEY = "sk-test-0123456789abcdef"


def _provider(
    server: LLMServer,
    *,
    key: str | None = KEY,
    keyring: str | None = "deepseek",
    timeout: float = 5,
) -> OpenAICompatProvider:
    config = ProviderConfig(
        name="deepseek",
        kind="openai_compat",
        base_url=server.base_url,
        model="test-model",
        timeout_s=timeout,
        keyring_name=keyring,
    )
    return OpenAICompatProvider(config, lambda _name: key)


def _request(text: str = "hi", **kwargs: object) -> ChatRequest:
    return ChatRequest(messages=(Message("user", text),), **kwargs)  # type: ignore[arg-type]


def _completion(content: str | None, **extra: object) -> bytes:
    message: dict[str, object] = {"role": "assistant", "content": content}
    message.update({k: v for k, v in extra.items() if k == "tool_calls"})
    body = {
        "model": "test-model-001",
        "choices": [{"message": message, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 7, "completion_tokens": 3},
    }
    return json.dumps(body).encode()


def _sse(*events: object) -> list[str]:
    out = []
    for event in events:
        if isinstance(event, str) and event.startswith(":"):
            out.append(f"{event}\n\n")  # an SSE comment line, sent as is
            continue
        text = event if isinstance(event, str) else json.dumps(event)
        out.append(f"data: {text}\n\n")
    return out


# ----------------------------------------------------------------------- chat ----
def test_chat_returns_text_and_usage(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_completion("hello")))
    response = _provider(llm_server).chat(_request())
    assert response.text == "hello"
    assert response.provider == "deepseek"
    assert response.model == "test-model-001"
    assert response.finish_reason == "stop"
    assert response.usage is not None and response.usage.prompt_tokens == 7


def test_request_goes_to_chat_completions_with_header_auth_only(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_completion("ok")))
    _provider(llm_server).chat(_request("question", temperature=0.2, max_tokens=50))
    sent = llm_server.requests[0]
    assert sent.path == "/v1/chat/completions"
    assert sent.headers["authorization"] == f"Bearer {KEY}"
    assert KEY not in sent.path
    payload = json.loads(sent.body)
    assert payload["model"] == "test-model"
    assert payload["stream"] is False
    assert payload["temperature"] == 0.2
    assert payload["max_tokens"] == 50
    assert payload["messages"] == [{"role": "user", "content": "question"}]


def test_provider_without_keyring_name_sends_no_authorization(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_completion("local")))
    _provider(llm_server, keyring=None, key=None).chat(_request())
    assert "authorization" not in llm_server.requests[0].headers


def test_missing_key_fails_before_any_network_call(llm_server: LLMServer) -> None:
    with pytest.raises(ProviderError, match="kellu keys set deepseek") as exc:
        _provider(llm_server, key=None).chat(_request())
    assert exc.value.retryable is False
    assert llm_server.requests == []


@pytest.mark.parametrize("bad", ["abc\r\nX-Evil: 1", "has space", "tab\tkey"])
def test_malformed_key_is_rejected_to_prevent_header_injection(
    llm_server: LLMServer, bad: str
) -> None:
    with pytest.raises(ProviderError, match="malformed"):
        _provider(llm_server, key=bad).chat(_request())
    assert llm_server.requests == []


def test_key_with_trailing_newline_is_accepted(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_completion("ok")))
    _provider(llm_server, key=KEY + "\n").chat(_request())
    assert llm_server.requests[0].headers["authorization"] == f"Bearer {KEY}"


def test_tools_are_sent_and_tool_calls_are_parsed(llm_server: LLMServer) -> None:
    calls = [
        {
            "id": "call_1",
            "type": "function",
            "function": {"name": "read_file", "arguments": '{"path": "a.py"}'},
        }
    ]
    llm_server.queue(Reply(body=_completion(None, tool_calls=calls)))
    tool = ToolSpec("read_file", "Read a file", {"type": "object", "properties": {}})
    response = _provider(llm_server).chat(_request(tools=(tool,)))
    assert response.text == ""
    assert response.tool_calls == (ToolCall("call_1", "read_file", {"path": "a.py"}),)
    sent = json.loads(llm_server.requests[0].body)
    assert sent["tools"][0]["function"]["name"] == "read_file"


def test_assistant_tool_calls_and_tool_results_are_serialized(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_completion("done")))
    call = ToolCall("call_1", "read_file", {"path": "a.py"})
    request = ChatRequest(
        messages=(
            Message("user", "read it"),
            Message("assistant", "", tool_calls=(call,)),
            Message("tool", "contents", tool_call_id="call_1"),
        )
    )
    _provider(llm_server).chat(request)
    messages = json.loads(llm_server.requests[0].body)["messages"]
    assert messages[1]["tool_calls"][0]["function"]["arguments"] == '{"path": "a.py"}'
    assert messages[2] == {"role": "tool", "content": "contents", "tool_call_id": "call_1"}


@pytest.mark.parametrize("arguments", ["not json", "[1, 2]", '"text"'])
def test_invalid_tool_arguments_are_rejected(llm_server: LLMServer, arguments: str) -> None:
    calls = [{"id": "c", "type": "function", "function": {"name": "t", "arguments": arguments}}]
    llm_server.queue(Reply(body=_completion(None, tool_calls=calls)))
    with pytest.raises(ProviderError, match="malformed"):
        _provider(llm_server).chat(_request())


def test_tool_name_with_unsafe_characters_is_rejected(llm_server: LLMServer) -> None:
    calls = [{"id": "c", "type": "function", "function": {"name": "rm -rf /", "arguments": "{}"}}]
    llm_server.queue(Reply(body=_completion(None, tool_calls=calls)))
    with pytest.raises(ProviderError, match="invalid name"):
        _provider(llm_server).chat(_request())


def test_content_given_as_parts_is_joined(llm_server: LLMServer) -> None:
    body = {"choices": [{"message": {"content": [{"type": "text", "text": "a"}, {"text": "b"}]}}]}
    llm_server.queue(Reply(body=json.dumps(body).encode()))
    assert _provider(llm_server).chat(_request()).text == "ab"


# --------------------------------------------------------------------- errors ----
@pytest.mark.parametrize(
    ("status", "retryable"),
    [
        (400, False),
        (401, False),
        (403, False),
        (404, False),
        (408, True),
        (429, True),
        (500, True),
        (503, True),
    ],
)
def test_http_status_maps_to_retryable_flag(
    llm_server: LLMServer, status: int, retryable: bool
) -> None:
    llm_server.queue(Reply(status=status, body=b'{"error": {"message": "nope"}}'))
    with pytest.raises(ProviderError) as exc:
        _provider(llm_server).chat(_request())
    assert exc.value.status == status
    assert exc.value.retryable is retryable
    assert "nope" in exc.value.reason


def test_error_text_never_contains_the_key_or_control_characters(llm_server: LLMServer) -> None:
    body = json.dumps({"error": {"message": f"bad key {KEY}\x1b[31m!"}}).encode()
    llm_server.queue(Reply(status=401, body=body))
    with pytest.raises(ProviderError) as exc:
        _provider(llm_server).chat(_request())
    assert KEY not in str(exc.value)
    assert "\x1b" not in str(exc.value)
    assert "***" in str(exc.value)


def test_error_object_inside_http_200_is_an_error(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=b'{"error": {"message": "overloaded", "code": 503}}'))
    with pytest.raises(ProviderError) as exc:
        _provider(llm_server).chat(_request())
    assert exc.value.retryable is True
    assert exc.value.status == 503


@pytest.mark.parametrize("body", [b"not json", b"[]", b'{"choices": []}', b'{"choices": [{}]}'])
def test_malformed_responses_are_not_retryable(llm_server: LLMServer, body: bytes) -> None:
    llm_server.queue(Reply(body=body))
    with pytest.raises(ProviderError) as exc:
        _provider(llm_server).chat(_request())
    assert exc.value.retryable is False


def test_redirects_are_refused_and_not_followed(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(status=302, headers={"Location": "http://127.0.0.1:1/steal"}))
    with pytest.raises(ProviderError, match="redirect refused") as exc:
        _provider(llm_server).chat(_request())
    assert exc.value.retryable is False
    assert len(llm_server.requests) == 1


def test_oversized_response_is_rejected(
    llm_server: LLMServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(http, "MAX_RESPONSE_BYTES", 200)
    llm_server.queue(Reply(body=_completion("x" * 1000)))
    with pytest.raises(ProviderError, match="too large"):
        _provider(llm_server).chat(_request())


def test_timeout_is_retryable(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(body=_completion("late"), delay_s=1.0))
    with pytest.raises(ProviderError, match="timed out") as exc:
        _provider(llm_server, timeout=0.2).chat(_request())
    assert exc.value.retryable is True


def test_unreachable_server_is_retryable() -> None:
    config = ProviderConfig("x", "openai_compat", "http://127.0.0.1:1/v1", "m", 2.0, None)
    with pytest.raises(ProviderError) as exc:
        OpenAICompatProvider(config, lambda _n: None).chat(_request())
    assert exc.value.retryable is True


@pytest.mark.parametrize("url", ["http://api.example.com/v1", "file:///etc/passwd", "ftp://x/y"])
def test_transport_refuses_unsafe_endpoints_even_if_configured(url: str) -> None:
    config = ProviderConfig("x", "openai_compat", url, "m", 2.0, None)
    with pytest.raises(ProviderError, match="unsafe endpoint") as exc:
        OpenAICompatProvider(config, lambda _n: None).chat(_request())
    assert exc.value.retryable is False


# ------------------------------------------------------------------- streaming ----
def test_stream_yields_text_then_a_final_response(llm_server: LLMServer) -> None:
    llm_server.queue(
        Reply(
            sse=_sse(
                {"model": "m1", "choices": [{"delta": {"content": "Hel"}}]},
                ": OPENROUTER PROCESSING",
                {"choices": [{"delta": {"content": "lo"}}]},
                {
                    "choices": [{"delta": {}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 4, "completion_tokens": 2},
                },
                "[DONE]",
            )
        )
    )
    events = list(_provider(llm_server).stream(_request()))
    assert [e.text for e in events if e.kind == "text"] == ["Hel", "lo"]
    final = events[-1]
    assert final.kind == "done" and final.response is not None
    assert final.response.text == "Hello"
    assert final.response.finish_reason == "stop"
    assert final.response.model == "m1"
    assert final.response.usage is not None and final.response.usage.completion_tokens == 2
    assert json.loads(llm_server.requests[0].body)["stream"] is True


def test_stream_accumulates_tool_call_fragments(llm_server: LLMServer) -> None:
    def delta(**fn: object) -> dict[str, object]:
        return {"choices": [{"delta": {"tool_calls": [{"index": 0, **fn}]}}]}

    llm_server.queue(
        Reply(
            sse=_sse(
                delta(id="call_9", function={"name": "read_file", "arguments": ""}),
                delta(function={"arguments": '{"path":'}),
                delta(function={"arguments": ' "a.py"}'}),
                {"choices": [{"delta": {}, "finish_reason": "tool_calls"}]},
                "[DONE]",
            )
        )
    )
    final = list(_provider(llm_server).stream(_request()))[-1].response
    assert final is not None
    assert final.tool_calls == (ToolCall("call_9", "read_file", {"path": "a.py"}),)
    assert final.finish_reason == "tool_calls"


def test_stream_error_event_raises(llm_server: LLMServer) -> None:
    llm_server.queue(
        Reply(
            sse=_sse(
                {"choices": [{"delta": {"content": "par"}}]},
                {"error": {"message": "boom", "code": 500}},
            )
        )
    )
    iterator = _provider(llm_server).stream(_request())
    assert next(iterator).text == "par"
    with pytest.raises(ProviderError, match="boom"):
        next(iterator)


def test_stream_that_ends_without_done_or_finish_reason_fails(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(sse=_sse({"choices": [{"delta": {"content": "cut"}}]})))
    with pytest.raises(ProviderError, match="ended unexpectedly"):
        list(_provider(llm_server).stream(_request()))


def test_stream_with_invalid_json_event_fails(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(sse=["data: {not json}\n\n"]))
    with pytest.raises(ProviderError, match="not JSON"):
        list(_provider(llm_server).stream(_request()))


def test_stream_http_error_is_raised_when_iterating(llm_server: LLMServer) -> None:
    llm_server.queue(Reply(status=429, body=b'{"error": {"message": "slow down"}}'))
    with pytest.raises(ProviderError) as exc:
        list(_provider(llm_server).stream(_request()))
    assert exc.value.retryable is True
