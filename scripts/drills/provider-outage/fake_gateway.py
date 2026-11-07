#!/usr/bin/env python3
"""Fake OpenAI-compatible gateway for the outage drill (stdlib only, loopback only).

POST /v1/chat/completions answers `503 no_capacity` until DOWN_SECS after the FIRST request, then
a healthy SSE reply. GET /v1/models lists one model. Every request is appended to a jsonl log.

usage: fake_gateway.py PORT DOWN_SECS LOG [retry_after]
"""
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
DOWN_SECS = float(sys.argv[2])
LOG = sys.argv[3]
RETRY_AFTER = sys.argv[4] if len(sys.argv) > 4 else None

lock = threading.Lock()
first_request_at = None

ANSWER = (
    'data: {"choices":[{"index":0,"delta":{"content":"DRILL-OK the gateway is back"},"finish_reason":null}]}\n\n'
    'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n'
    'data: {"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":6,"total_tokens":15}}\n\n'
    "data: [DONE]\n\n"
)
NO_CAPACITY = json.dumps(
    {"error": {"code": "no_capacity", "type": "service_unavailable",
               "message": "No capacity available for drill/flash"}}
)


def log(entry):
    with lock, open(LOG, "a") as f:
        f.write(json.dumps(entry) + "\n")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def _send(self, status, body, ctype="application/json", extra=None):
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        log({"t": time.time(), "method": "GET", "path": self.path})
        if self.path.rstrip("/").endswith("/models"):
            self._send(200, json.dumps({"object": "list", "data": [{"id": "flash", "object": "model"}]}))
        else:
            self._send(404, "{}")

    def do_POST(self):
        global first_request_at
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length)
        with lock:
            now = time.time()
            if first_request_at is None:
                first_request_at = now
            down = now - first_request_at < DOWN_SECS
        log({"t": now, "method": "POST", "path": self.path, "down": down,
             "since_first": round(now - first_request_at, 1), "bytes": len(body)})
        if down:
            extra = {"Retry-After": RETRY_AFTER} if RETRY_AFTER else None
            self._send(503, NO_CAPACITY, extra=extra)
        else:
            self._send(200, ANSWER, ctype="text/event-stream")


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
