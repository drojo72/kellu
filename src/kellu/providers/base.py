"""The provider interface."""

from __future__ import annotations

from collections.abc import Callable, Iterator
from typing import Protocol

from kellu.providers.types import ChatRequest, ChatResponse, StreamEvent

# Maps a keyring name (for example "openrouter") to its secret, or None when it is missing.
# The real implementation is the keyring vault (milestone M3); tests inject a fake.
KeyLookup = Callable[[str], str | None]


class Provider(Protocol):
    """A model provider. Implementations never log or raise messages containing secrets."""

    name: str
    supports_tools: bool

    def chat(self, request: ChatRequest) -> ChatResponse:
        """Send ``request`` and return the complete answer."""
        ...

    def stream(self, request: ChatRequest) -> Iterator[StreamEvent]:
        """Send ``request`` and yield text deltas, then one final ``done`` event."""
        ...
