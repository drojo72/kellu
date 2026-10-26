"""HTTP transport for providers (standard library only).

Security properties:

* Endpoints must pass :func:`kellu.common.net.endpoint_problem` (https, or http for localhost).
* Redirects are refused, so an API key is never replayed to another host.
* Response bodies, individual SSE lines and total stream duration are bounded.
* Error messages are scrubbed: control characters removed, the API key replaced, length limited.
* Keys travel only in headers supplied by the caller, never in the URL.
"""

from __future__ import annotations

import http.client
import json
import ssl
import time
import urllib.error
import urllib.request
from collections.abc import Callable, Iterator, Mapping
from contextlib import closing
from email.message import Message as HeaderMessage
from typing import IO, Any

from kellu.common.errors import ProviderError
from kellu.common.net import endpoint_problem
from kellu.common.text import strip_control_chars, truncate

MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_LINE_BYTES = 1024 * 1024
MAX_STREAM_SECONDS = 600.0
ERROR_BODY_BYTES = 4096
_READ_CHUNK = 64 * 1024

Clock = Callable[[], float]


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Never follow redirects: surface them as errors instead."""

    def redirect_request(
        self,
        req: urllib.request.Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: HeaderMessage,
        newurl: str,
    ) -> urllib.request.Request | None:
        return None


def scrub(text: str, secret: str | None) -> str:
    """Make ``text`` safe to show: no control characters, no key, single line, bounded."""
    cleaned = strip_control_chars(text)
    if secret and len(secret) >= 8:
        cleaned = cleaned.replace(secret, "***")
    return truncate(" ".join(cleaned.split()), 300)


def _message_from_body(body: bytes, secret: str | None) -> str:
    text = body.decode("utf-8", errors="replace")
    try:
        data = json.loads(text)
    except ValueError:
        return scrub(text, secret)
    error = data.get("error") if isinstance(data, dict) else None
    if isinstance(error, dict) and isinstance(error.get("message"), str):
        return scrub(error["message"], secret)
    if isinstance(error, str):
        return scrub(error, secret)
    return scrub(text, secret)


def retryable_status(status: int) -> bool:
    """Whether another attempt or another provider may succeed."""
    return status in (408, 409, 425, 429) or status >= 500


def _open(
    provider: str,
    url: str,
    headers: Mapping[str, str],
    body: bytes,
    timeout: float,
    secret: str | None,
) -> http.client.HTTPResponse:
    problem = endpoint_problem(url)
    if problem is not None:
        raise ProviderError(provider, f"unsafe endpoint: {problem}", retryable=False)
    request = urllib.request.Request(  # noqa: S310 - scheme validated by endpoint_problem
        url, data=body, headers=dict(headers), method="POST"
    )
    opener = urllib.request.build_opener(_NoRedirect)
    try:
        return opener.open(request, timeout=timeout)  # type: ignore[no-any-return]  # noqa: S310
    except urllib.error.HTTPError as exc:
        status = exc.code
        try:
            detail = exc.read(ERROR_BODY_BYTES)
        except OSError:
            detail = b""
        finally:
            exc.close()
        if 300 <= status < 400:
            raise ProviderError(
                provider,
                "redirect refused (has the endpoint moved?)",
                status=status,
                retryable=False,
            ) from exc
        message = _message_from_body(detail, secret) or str(exc.reason)
        raise ProviderError(
            provider, message, status=status, retryable=retryable_status(status)
        ) from exc
    except urllib.error.URLError as exc:
        reason = exc.reason
        if isinstance(reason, TimeoutError):
            raise ProviderError(provider, "request timed out", retryable=True) from exc
        if isinstance(reason, ssl.SSLError):
            raise ProviderError(
                provider, f"TLS error: {scrub(str(reason), secret)}", retryable=False
            ) from exc
        raise ProviderError(
            provider, f"network error: {scrub(str(reason), secret)}", retryable=True
        ) from exc
    except TimeoutError as exc:
        raise ProviderError(provider, "request timed out", retryable=True) from exc
    except OSError as exc:
        raise ProviderError(
            provider, f"network error: {scrub(str(exc), secret)}", retryable=True
        ) from exc


def _read_limited(provider: str, response: http.client.HTTPResponse, limit: int) -> bytes:
    chunks: list[bytes] = []
    total = 0
    try:
        while True:
            chunk = response.read(_READ_CHUNK)
            if not chunk:
                break
            total += len(chunk)
            if total > limit:
                raise ProviderError(provider, "response too large", retryable=False)
            chunks.append(chunk)
    except TimeoutError as exc:
        raise ProviderError(provider, "timed out while reading the response") from exc
    except (OSError, http.client.HTTPException) as exc:
        raise ProviderError(provider, "connection lost while reading the response") from exc
    return b"".join(chunks)


def post_json(
    *,
    provider: str,
    url: str,
    headers: Mapping[str, str],
    payload: Mapping[str, Any],
    timeout: float,
    secret: str | None,
) -> dict[str, Any]:
    """POST ``payload`` as JSON and return the decoded JSON object."""
    body = json.dumps(payload).encode("utf-8")
    with closing(_open(provider, url, headers, body, timeout, secret)) as response:
        raw = _read_limited(provider, response, MAX_RESPONSE_BYTES)
    try:
        data = json.loads(raw)
    except ValueError as exc:
        raise ProviderError(
            provider, "malformed response: not JSON", status=200, retryable=False
        ) from exc
    if not isinstance(data, dict):
        raise ProviderError(
            provider, "malformed response: expected an object", status=200, retryable=False
        )
    return data


def post_sse(
    *,
    provider: str,
    url: str,
    headers: Mapping[str, str],
    payload: Mapping[str, Any],
    timeout: float,
    secret: str | None,
    clock: Clock = time.monotonic,
) -> Iterator[str]:
    """POST ``payload`` and yield the ``data`` field of each server-sent event."""
    body = json.dumps(payload).encode("utf-8")
    response = _open(provider, url, headers, body, timeout, secret)
    deadline = clock() + MAX_STREAM_SECONDS
    total = 0
    data_lines: list[str] = []
    try:
        while True:
            try:
                raw = response.readline(MAX_LINE_BYTES + 1)
            except TimeoutError as exc:
                raise ProviderError(provider, "timed out while streaming") from exc
            except (OSError, http.client.HTTPException) as exc:
                raise ProviderError(provider, "connection lost while streaming") from exc
            if not raw:
                break
            total += len(raw)
            if len(raw) > MAX_LINE_BYTES:
                raise ProviderError(provider, "stream line too long", retryable=False)
            if total > MAX_RESPONSE_BYTES:
                raise ProviderError(provider, "response too large", retryable=False)
            if clock() > deadline:
                raise ProviderError(provider, "stream exceeded the maximum duration")
            line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
            if not line:
                if data_lines:
                    yield "\n".join(data_lines)
                    data_lines = []
                continue
            if line.startswith(":"):
                continue  # comment or keep-alive
            name, _, value = line.partition(":")
            if name == "data":
                data_lines.append(value[1:] if value.startswith(" ") else value)
        if data_lines:
            yield "\n".join(data_lines)
    finally:
        response.close()
