"""Client for the Google Gemini API (``generateContent``), used for Gemini and Gemma models.

The API key goes in the ``x-goog-api-key`` header, never in the URL. Gemma models do not accept a
system instruction, so system messages are folded into the first user turn for them.
"""

from __future__ import annotations

import json
from collections.abc import Iterator, Mapping
from typing import Any
from urllib.parse import quote

from kellu.common.errors import ProviderError
from kellu.providers import http
from kellu.providers._common import malformed, parse_usage, resolve_key
from kellu.providers.base import KeyLookup
from kellu.providers.types import ChatRequest, ChatResponse, ProviderConfig, StreamEvent, Usage


def _inline_system(model: str) -> bool:
    return model.lower().removeprefix("models/").startswith("gemma")


def _text_of(candidate: Mapping[str, Any]) -> str:
    content = candidate.get("content")
    parts = content.get("parts") if isinstance(content, dict) else None
    if not isinstance(parts, list):
        return ""
    pieces = [
        p["text"]
        for p in parts
        if isinstance(p, dict) and isinstance(p.get("text"), str) and not p.get("thought")
    ]
    return "".join(pieces)


class GoogleProvider:
    """Text chat over ``models/{model}:generateContent``. Tool calling is not wired yet."""

    supports_tools = False

    def __init__(self, config: ProviderConfig, key_lookup: KeyLookup) -> None:
        self.name = config.name
        self._config = config
        self._key_lookup = key_lookup

    # ------------------------------------------------------------------ request ----
    def _prepare(self, request: ChatRequest) -> tuple[dict[str, str], dict[str, Any], str | None]:
        if request.tools:
            raise ProviderError(self.name, "tool calling is not supported yet", retryable=False)
        key = resolve_key(self._config, self._key_lookup)
        headers = {"Content-Type": "application/json"}
        if key is not None:
            headers["x-goog-api-key"] = key

        system = [m.content for m in request.messages if m.role == "system" and m.content]
        turns: list[dict[str, Any]] = []
        for message in request.messages:
            if message.role == "system":
                continue
            if message.role == "tool" or message.tool_calls:
                raise ProviderError(
                    self.name, "tool messages are not supported yet", retryable=False
                )
            role = "user" if message.role == "user" else "model"
            turns.append({"role": role, "parts": [{"text": message.content}]})
        if not turns:
            raise ProviderError(
                self.name, "the request has no user or assistant turns", retryable=False
            )

        payload: dict[str, Any] = {"contents": turns}
        if system:
            text = "\n\n".join(system)
            if _inline_system(self._config.model):
                first = turns[0]
                if first["role"] == "user":
                    first["parts"][0]["text"] = f"{text}\n\n{first['parts'][0]['text']}"
                else:
                    turns.insert(0, {"role": "user", "parts": [{"text": text}]})
            else:
                payload["systemInstruction"] = {"parts": [{"text": text}]}
        generation: dict[str, Any] = {}
        if request.temperature is not None:
            generation["temperature"] = request.temperature
        if request.max_tokens is not None:
            generation["maxOutputTokens"] = request.max_tokens
        if generation:
            payload["generationConfig"] = generation
        return headers, payload, key

    def _url(self, method: str, query: str = "") -> str:
        model = quote(self._config.model.removeprefix("models/"), safe="")
        return f"{self._config.base_url.rstrip('/')}/models/{model}:{method}{query}"

    # ------------------------------------------------------------------ public ----
    def chat(self, request: ChatRequest) -> ChatResponse:
        headers, payload, key = self._prepare(request)
        data = http.post_json(
            provider=self.name,
            url=self._url("generateContent"),
            headers=headers,
            payload=payload,
            timeout=self._config.timeout_s,
            secret=key,
        )
        self._raise_if_error(data, key)
        candidate = self._first_candidate(data)
        version = data.get("modelVersion")
        reason = candidate.get("finishReason")
        return ChatResponse(
            text=_text_of(candidate),
            provider=self.name,
            model=version if isinstance(version, str) else None,
            finish_reason=reason if isinstance(reason, str) else None,
            usage=self._usage(data),
        )

    def stream(self, request: ChatRequest) -> Iterator[StreamEvent]:
        headers, payload, key = self._prepare(request)
        headers["Accept"] = "text/event-stream"
        text_parts: list[str] = []
        finish: str | None = None
        usage: Usage | None = None
        model: str | None = None
        received = False
        for data_text in http.post_sse(
            provider=self.name,
            url=self._url("streamGenerateContent", "?alt=sse"),
            headers=headers,
            payload=payload,
            timeout=self._config.timeout_s,
            secret=key,
        ):
            try:
                chunk = json.loads(data_text)
            except ValueError as exc:
                raise malformed(self.name, "stream event is not JSON") from exc
            if not isinstance(chunk, dict):
                raise malformed(self.name, "stream event is not an object")
            self._raise_if_error(chunk, key)
            received = True
            if isinstance(chunk.get("modelVersion"), str):
                model = chunk["modelVersion"]
            usage = self._usage(chunk) or usage
            candidates = chunk.get("candidates")
            if not isinstance(candidates, list) or not candidates:
                self._raise_if_blocked(chunk)
                continue
            candidate = candidates[0]
            if not isinstance(candidate, dict):
                raise malformed(self.name, "candidate is not an object")
            piece = _text_of(candidate)
            if piece:
                text_parts.append(piece)
                yield StreamEvent(kind="text", text=piece)
            if isinstance(candidate.get("finishReason"), str):
                finish = candidate["finishReason"]
        if not received:
            raise ProviderError(self.name, "stream ended without data")
        yield StreamEvent(
            kind="done",
            response=ChatResponse(
                text="".join(text_parts),
                provider=self.name,
                model=model,
                finish_reason=finish,
                usage=usage,
            ),
        )

    # ----------------------------------------------------------------- helpers ----
    def _raise_if_error(self, data: Mapping[str, Any], key: str | None) -> None:
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

    def _raise_if_blocked(self, data: Mapping[str, Any]) -> None:
        feedback = data.get("promptFeedback")
        reason = feedback.get("blockReason") if isinstance(feedback, dict) else None
        if reason:
            raise ProviderError(
                self.name, f"prompt blocked by the provider ({reason})", retryable=False
            )

    def _first_candidate(self, data: Mapping[str, Any]) -> Mapping[str, Any]:
        candidates = data.get("candidates")
        if not isinstance(candidates, list) or not candidates:
            self._raise_if_blocked(data)
            raise malformed(self.name, "no candidates")
        candidate = candidates[0]
        if not isinstance(candidate, dict):
            raise malformed(self.name, "candidate is not an object")
        return candidate

    @staticmethod
    def _usage(data: Mapping[str, Any]) -> Usage | None:
        return parse_usage(data.get("usageMetadata"), "promptTokenCount", "candidatesTokenCount")
