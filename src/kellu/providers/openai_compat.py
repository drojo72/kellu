"""Client for OpenAI-compatible chat-completions APIs (OpenRouter, DeepSeek, xAI, local)."""

from __future__ import annotations

import json
from collections.abc import Iterator, Mapping
from typing import Any

from kellu.common.errors import ProviderError
from kellu.providers import http
from kellu.providers._common import (
    MAX_TOOL_CALLS,
    make_tool_call,
    malformed,
    parse_usage,
    resolve_key,
)
from kellu.providers.base import KeyLookup
from kellu.providers.types import (
    ChatRequest,
    ChatResponse,
    Message,
    ProviderConfig,
    StreamEvent,
    ToolCall,
    Usage,
)


def _message_payload(message: Message) -> dict[str, Any]:
    payload: dict[str, Any] = {"role": message.role, "content": message.content}
    if message.tool_calls:
        payload["tool_calls"] = [
            {
                "id": call.id,
                "type": "function",
                "function": {"name": call.name, "arguments": json.dumps(call.arguments)},
            }
            for call in message.tool_calls
        ]
    if message.role == "tool":
        payload["tool_call_id"] = message.tool_call_id
    return payload


def _content_text(content: object) -> str:
    """Providers return a string, ``None`` (tool calls only) or a list of content parts."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = [p.get("text", "") for p in content if isinstance(p, dict)]
        return "".join(p for p in parts if isinstance(p, str))
    return ""


class OpenAICompatProvider:
    """``POST {base_url}/chat/completions`` with Bearer authentication."""

    supports_tools = True

    def __init__(self, config: ProviderConfig, key_lookup: KeyLookup) -> None:
        self.name = config.name
        self._config = config
        self._key_lookup = key_lookup

    # ------------------------------------------------------------------ request ----
    def _prepare(
        self, request: ChatRequest, *, stream: bool
    ) -> tuple[dict[str, str], dict[str, Any], str | None]:
        key = resolve_key(self._config, self._key_lookup)
        headers = {"Content-Type": "application/json", "Accept": "application/json"}
        if key is not None:
            headers["Authorization"] = f"Bearer {key}"
        payload: dict[str, Any] = {
            "model": self._config.model,
            "messages": [_message_payload(m) for m in request.messages],
            "stream": stream,
        }
        if request.tools:
            payload["tools"] = [
                {
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": dict(t.parameters),
                    },
                }
                for t in request.tools
            ]
        if request.temperature is not None:
            payload["temperature"] = request.temperature
        if request.max_tokens is not None:
            payload["max_tokens"] = request.max_tokens
        return headers, payload, key

    @property
    def _url(self) -> str:
        return f"{self._config.base_url.rstrip('/')}/chat/completions"

    # ------------------------------------------------------------------ public ----
    def chat(self, request: ChatRequest) -> ChatResponse:
        headers, payload, key = self._prepare(request, stream=False)
        data = http.post_json(
            provider=self.name,
            url=self._url,
            headers=headers,
            payload=payload,
            timeout=self._config.timeout_s,
            secret=key,
        )
        self._raise_if_error(data, key)
        choices = data.get("choices")
        if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
            raise malformed(self.name, "no choices")
        choice: Mapping[str, Any] = choices[0]
        message = choice.get("message")
        if not isinstance(message, dict):
            raise malformed(self.name, "choice without a message")
        raw_calls = message.get("tool_calls") or []
        if not isinstance(raw_calls, list):
            raise malformed(self.name, "tool_calls is not a list")
        if len(raw_calls) > MAX_TOOL_CALLS:
            raise malformed(self.name, "too many tool calls")
        calls = tuple(self._tool_call(item) for item in raw_calls)
        model = data.get("model")
        finish = choice.get("finish_reason")
        return ChatResponse(
            text=_content_text(message.get("content")),
            provider=self.name,
            model=model if isinstance(model, str) else None,
            tool_calls=calls,
            finish_reason=finish if isinstance(finish, str) else None,
            usage=parse_usage(data.get("usage"), "prompt_tokens", "completion_tokens"),
        )

    def stream(self, request: ChatRequest) -> Iterator[StreamEvent]:
        headers, payload, key = self._prepare(request, stream=True)
        headers["Accept"] = "text/event-stream"
        text_parts: list[str] = []
        partial: dict[int, dict[str, Any]] = {}
        finish: str | None = None
        usage: Usage | None = None
        model: str | None = None
        done = False
        for data_text in http.post_sse(
            provider=self.name,
            url=self._url,
            headers=headers,
            payload=payload,
            timeout=self._config.timeout_s,
            secret=key,
        ):
            if data_text.strip() == "[DONE]":
                done = True
                break
            try:
                chunk = json.loads(data_text)
            except ValueError as exc:
                raise malformed(self.name, "stream event is not JSON") from exc
            if not isinstance(chunk, dict):
                raise malformed(self.name, "stream event is not an object")
            self._raise_if_error(chunk, key)
            if isinstance(chunk.get("model"), str):
                model = chunk["model"]
            usage = parse_usage(chunk.get("usage"), "prompt_tokens", "completion_tokens") or usage
            choices = chunk.get("choices")
            if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
                continue
            delta = choices[0].get("delta")
            if isinstance(delta, dict):
                piece = _content_text(delta.get("content"))
                if piece:
                    text_parts.append(piece)
                    yield StreamEvent(kind="text", text=piece)
                for item in delta.get("tool_calls") or []:
                    self._accumulate_tool_delta(partial, item)
            reason = choices[0].get("finish_reason")
            if isinstance(reason, str):
                finish = reason
        if not done and finish is None:
            raise ProviderError(self.name, "stream ended unexpectedly")
        calls = tuple(
            make_tool_call(self.name, p.get("id"), p.get("name"), "".join(p["args"]))
            for _, p in sorted(partial.items())
        )
        yield StreamEvent(
            kind="done",
            response=ChatResponse(
                text="".join(text_parts),
                provider=self.name,
                model=model,
                tool_calls=calls,
                finish_reason=finish,
                usage=usage,
            ),
        )

    # ----------------------------------------------------------------- helpers ----
    def _raise_if_error(self, data: Mapping[str, Any], key: str | None) -> None:
        """Some gateways return HTTP 200 with an ``error`` object, also inside streams."""
        error = data.get("error")
        if not error:
            return
        message = error.get("message") if isinstance(error, dict) else error
        code = error.get("code") if isinstance(error, dict) else None
        status = code if isinstance(code, int) and not isinstance(code, bool) else 0
        raise ProviderError(
            self.name,
            http.scrub(str(message), key),
            status=status,
            retryable=http.retryable_status(status) if status else True,
        )

    def _tool_call(self, item: object) -> ToolCall:
        if not isinstance(item, dict) or not isinstance(item.get("function"), dict):
            raise malformed(self.name, "tool call without a function")
        function = item["function"]
        return make_tool_call(
            self.name, item.get("id"), function.get("name"), function.get("arguments")
        )

    def _accumulate_tool_delta(self, partial: dict[int, dict[str, Any]], item: object) -> None:
        if not isinstance(item, dict):
            raise malformed(self.name, "tool call delta is not an object")
        index = item.get("index", 0)
        if not isinstance(index, int) or isinstance(index, bool) or not 0 <= index < MAX_TOOL_CALLS:
            raise malformed(self.name, "tool call delta with an invalid index")
        entry = partial.setdefault(index, {"id": None, "name": None, "args": []})
        if isinstance(item.get("id"), str):
            entry["id"] = item["id"]
        function = item.get("function")
        if isinstance(function, dict):
            if isinstance(function.get("name"), str):
                entry["name"] = function["name"]
            if isinstance(function.get("arguments"), str):
                entry["args"].append(function["arguments"])
