"""Endpoint URL rules shared by configuration validation and the HTTP transport."""

from __future__ import annotations

from urllib.parse import urlsplit

LOCAL_HOSTS = frozenset({"localhost", "127.0.0.1", "::1"})


def endpoint_problem(url: str) -> str | None:
    """Return why ``url`` is not an acceptable provider endpoint, or ``None`` if it is.

    Endpoints must be absolute http(s) URLs without embedded credentials. Plain http is accepted
    only for localhost (local model runtimes); everything else must use https.
    """
    try:
        parts = urlsplit(url)
        host = parts.hostname
    except ValueError:
        return "invalid URL"
    if parts.scheme not in {"https", "http"} or not host:
        return "expected an absolute http(s) URL"
    if parts.username or parts.password:
        return "credentials in URLs are not allowed"
    if parts.scheme == "http" and host not in LOCAL_HOSTS:
        return "plain http is only allowed for localhost; use https"
    return None
