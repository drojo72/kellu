"""Helpers shared by the provider clients."""

from __future__ import annotations

import json
import re
from collections.abc import Mapping
from typing import Any

from kellu.common.errors import ProviderError
from kellu.providers.base import KeyLookup
from kellu.providers.types import ProviderConfig, ToolCall, Usage

MAX_TOOL_CALLS = 32
_TOOL_NAME_RE = re.compile(r"^[A-Za-z0-9_.-]{1,64}$")


def malformed(provider: str, detail: str) -> ProviderError:
    """A response that does not look like what the API promises (not worth retrying)."""
    return ProviderError(provider, f"malformed response: {detail}", status=200, retryable=False)


def resolve_key(config: ProviderConfig, lookup: KeyLookup) -> str | None:
    """Return the API key, ``None`` for providers without authentication.

    Raises ``ProviderError`` if a key is required but missing or unsafe to put in a header.
    """
    if config.keyring_name is None:
        return None
    raw = lookup(config.keyring_name)
    key = raw.strip() if raw else ""
    if not key:
        raise ProviderError(
            config.name,
            f"no API key for '{config.keyring_name}'; run `kellu keys set {config.keyring_name}`",
            retryable=False,
        )
    if any(ch.isspace() or ord(ch) < 32 or ord(ch) == 127 for ch in key):
        raise ProviderError(config.name, "the stored API key is malformed", retryable=False)
    return key


def parse_arguments(provider: str, raw: object) -> dict[str, Any]:
    """Turn tool-call arguments (a JSON string or an object) into a dict."""
    if isinstance(raw, dict):
        return raw
    if raw is None or (isinstance(raw, str) and not raw.strip()):
        return {}
    if not isinstance(raw, str):
        raise malformed(provider, "tool call arguments have an unexpected type")
    try:
        parsed = json.loads(raw)
    except ValueError as exc:
        raise malformed(provider, "tool call arguments are not valid JSON") from exc
    if not isinstance(parsed, dict):
        raise malformed(provider, "tool call arguments must be a JSON object")
    return parsed


def make_tool_call(provider: str, call_id: object, name: object, arguments: object) -> ToolCall:
    """Validate and build a :class:`ToolCall` from untrusted provider data."""
    if not isinstance(name, str) or not _TOOL_NAME_RE.match(name):
        raise malformed(provider, "tool call with an invalid name")
    ident = call_id if isinstance(call_id, str) and call_id else f"call_{name}"
    return ToolCall(id=ident[:128], name=name, arguments=parse_arguments(provider, arguments))


def parse_usage(raw: object, prompt_key: str, completion_key: str) -> Usage | None:
    """Read token counts from a provider usage object."""
    if not isinstance(raw, Mapping):
        return None

    def count(key: str) -> int | None:
        value = raw.get(key)
        return value if isinstance(value, int) and not isinstance(value, bool) else None

    return Usage(prompt_tokens=count(prompt_key), completion_tokens=count(completion_key))
