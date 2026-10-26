"""Build a provider client from its configuration."""

from __future__ import annotations

from collections.abc import Callable

from kellu.common.errors import ConfigError
from kellu.providers.base import KeyLookup, Provider
from kellu.providers.google import GoogleProvider
from kellu.providers.openai_compat import OpenAICompatProvider
from kellu.providers.types import ProviderConfig

_FACTORIES: dict[str, Callable[[ProviderConfig, KeyLookup], Provider]] = {
    "openai_compat": OpenAICompatProvider,
    "google": GoogleProvider,
}

# Single source of truth for the provider kinds the configuration accepts.
KINDS = frozenset(_FACTORIES)


def build_provider(config: ProviderConfig, key_lookup: KeyLookup) -> Provider:
    """Create the client for ``config``. Keys are looked up per request, never stored."""
    try:
        factory = _FACTORIES[config.kind]
    except KeyError:
        raise ConfigError(f"providers.{config.name}.kind: unknown kind {config.kind!r}") from None
    return factory(config, key_lookup)
