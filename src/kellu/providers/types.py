"""Provider-neutral data types. Standard library only."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any, Literal

Role = Literal["system", "user", "assistant", "tool"]


@dataclass(frozen=True)
class ProviderConfig:
    """One model provider. ``keyring_name`` names a keyring entry, never the secret itself.

    ``keyring_name = None`` means the endpoint needs no authentication (a local runtime).
    """

    name: str
    kind: str
    base_url: str
    model: str
    timeout_s: float
    keyring_name: str | None


@dataclass(frozen=True)
class ToolCall:
    """A tool invocation requested by the model. ``arguments`` is already parsed JSON."""

    id: str
    name: str
    arguments: Mapping[str, Any]


@dataclass(frozen=True)
class Message:
    """One conversation message."""

    role: Role
    content: str = ""
    tool_calls: tuple[ToolCall, ...] = ()
    tool_call_id: str | None = None

    def __post_init__(self) -> None:
        if self.role == "tool" and not self.tool_call_id:
            raise ValueError("a tool message needs a tool_call_id")


@dataclass(frozen=True)
class ToolSpec:
    """A tool the model may call. ``parameters`` is a JSON Schema object."""

    name: str
    description: str
    parameters: Mapping[str, Any]


@dataclass(frozen=True)
class ChatRequest:
    """A provider-neutral chat request."""

    messages: tuple[Message, ...]
    tools: tuple[ToolSpec, ...] = ()
    temperature: float | None = None
    max_tokens: int | None = None

    def __post_init__(self) -> None:
        if not self.messages:
            raise ValueError("a chat request needs at least one message")


@dataclass(frozen=True)
class Usage:
    """Token counts, when the provider reports them."""

    prompt_tokens: int | None = None
    completion_tokens: int | None = None


@dataclass(frozen=True)
class ChatResponse:
    """A complete model answer."""

    text: str
    provider: str
    model: str | None = None
    tool_calls: tuple[ToolCall, ...] = ()
    finish_reason: str | None = None
    usage: Usage | None = None


@dataclass(frozen=True)
class StreamEvent:
    """``text`` events carry a delta; the final ``done`` event carries the full response."""

    kind: Literal["text", "done"]
    text: str = ""
    response: ChatResponse | None = None
