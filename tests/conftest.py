"""Shared fixtures."""

from __future__ import annotations

import threading
import time
from collections import deque
from collections.abc import Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

import pytest


@pytest.fixture
def fake_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Isolate HOME and XDG variables so tests never touch the real user directories."""
    for var in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        monkeypatch.delenv(var, raising=False)
    monkeypatch.setenv("HOME", str(tmp_path))
    return tmp_path


# ------------------------------------------------------------ scripted HTTP server ----
@dataclass
class Reply:
    """One scripted response. ``sse`` sends raw text chunks without a Content-Length."""

    status: int = 200
    body: bytes = b""
    headers: dict[str, str] = field(default_factory=dict)
    sse: list[str] | None = None
    delay_s: float = 0.0


@dataclass(frozen=True)
class Recorded:
    """A request received by the scripted server."""

    path: str
    headers: dict[str, str]
    body: bytes


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"  # the body ends when the connection closes

    def do_POST(self) -> None:
        server: LLMServer = self.server  # type: ignore[assignment]
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        server.requests.append(
            Recorded(self.path, {k.lower(): v for k, v in self.headers.items()}, body)
        )
        reply = server.next_reply()
        if reply.delay_s:
            time.sleep(reply.delay_s)
        self.send_response(reply.status)
        for key, value in reply.headers.items():
            self.send_header(key, value)
        if reply.sse is not None:
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for chunk in reply.sse:
                self.wfile.write(chunk.encode("utf-8"))
                self.wfile.flush()
            return
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(reply.body)))
        self.end_headers()
        self.wfile.write(reply.body)

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
        pass


class LLMServer(ThreadingHTTPServer):
    """Local HTTP server that plays back scripted replies and records requests."""

    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), _Handler)
        self.requests: list[Recorded] = []
        self._replies: deque[Reply] = deque()
        self._lock = threading.Lock()

    def handle_error(self, request: Any, client_address: Any) -> None:
        pass  # clients that give up (timeouts) are expected in tests

    def queue(self, *replies: Reply) -> None:
        with self._lock:
            self._replies.extend(replies)

    def next_reply(self) -> Reply:
        with self._lock:
            return self._replies.popleft() if self._replies else Reply(status=500, body=b"{}")

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.server_address[1]}/v1"


@pytest.fixture
def llm_server() -> Iterator[LLMServer]:
    server = LLMServer()
    thread = threading.Thread(
        target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
    )
    thread.start()
    yield server
    server.shutdown()
    server.server_close()
    thread.join(timeout=5)
