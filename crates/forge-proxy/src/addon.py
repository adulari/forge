"""Forge's mitmproxy addon: stream every flow to disk as JSON, and apply live rules.

Loaded by `mitmdump -s`. Two files do all the talking, because the alternative — a control
socket or mitmproxy's own API — would mean Forge and mitmdump agreeing on a protocol that
outlives neither process:

  * CAPTURE (append-only JSONL, one flow per line, written when the response completes, or
    when the request fails). Forge reads it; nothing here ever reads it back.
  * RULES (JSON, re-read on every request). Forge rewrites the file to change behaviour, so
    rules take effect on the next request with no restart and no lost captures.

Bodies are truncated HERE rather than in Forge: a 200 MB video response should never reach
the capture file, let alone a model's context.
"""

import json
import os
import time

from mitmproxy import ctx, http

CAPTURE = os.environ.get("FORGE_PROXY_CAPTURE", "/tmp/forge-proxy-capture.jsonl")
RULES = os.environ.get("FORGE_PROXY_RULES", "")
# Per-body cap. Generous enough to hold a JSON API payload whole, small enough that a media
# response cannot bloat the capture file.
MAX_BODY = int(os.environ.get("FORGE_PROXY_MAX_BODY", "131072"))


def _rules():
    """Read the rules file. Missing or malformed → no rules, never an exception: a typo in a
    rules file must not take the proxy down and strand the device behind it."""
    if not RULES or not os.path.exists(RULES):
        return {}
    try:
        with open(RULES, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception as error:  # noqa: BLE001 - deliberately total
        ctx.log.warn(f"forge: ignoring unreadable rules file: {error}")
        return {}


def _body(raw):
    if not raw:
        return "", 0, False
    total = len(raw)
    clipped = raw[:MAX_BODY]
    try:
        text = clipped.decode("utf-8")
    except UnicodeDecodeError:
        # Binary. Say so rather than emitting mojibake the model would try to read.
        return f"<{total} bytes of binary>", total, True
    return text, total, total > MAX_BODY


def _matches(flow, pattern):
    return pattern and pattern.lower() in flow.request.pretty_url.lower()


def request(flow: http.HTTPFlow):
    rules = _rules()

    for pattern in rules.get("block", []):
        if _matches(flow, pattern):
            flow.response = http.Response.make(
                418, b"blocked by forge", {"x-forge-blocked": pattern}
            )
            return

    for rule in rules.get("set_request_headers", []):
        if _matches(flow, rule.get("url_contains", "")):
            for name, value in (rule.get("headers") or {}).items():
                flow.request.headers[name] = value

    for rule in rules.get("replace_request_body", []):
        if _matches(flow, rule.get("url_contains", "")):
            flow.request.text = rule.get("body", "")


def response(flow: http.HTTPFlow):
    rules = _rules()

    # A stubbed response replaces the real one AFTER it arrived, so the capture still records
    # what the server actually said — you can see both what the app got and what it would
    # have got.
    for rule in rules.get("stub_response", []):
        if _matches(flow, rule.get("url_contains", "")):
            flow.response = http.Response.make(
                int(rule.get("status", 200)),
                (rule.get("body") or "").encode("utf-8"),
                rule.get("headers") or {"content-type": "application/json"},
            )

    for rule in rules.get("set_response_headers", []):
        if flow.response is not None and _matches(flow, rule.get("url_contains", "")):
            for name, value in (rule.get("headers") or {}).items():
                flow.response.headers[name] = value

    _record(flow)


def error(flow: http.HTTPFlow):
    """A flow that never got a response is still evidence — a blocked host, a TLS refusal, a
    timeout. Dropping it would make the capture quietly incomplete."""
    _record(flow)


def _record(flow: http.HTTPFlow):
    request_text, request_total, request_clipped = _body(flow.request.raw_content)
    if flow.response is not None:
        response_text, response_total, response_clipped = _body(flow.response.raw_content)
        status = flow.response.status_code
        response_headers = dict(flow.response.headers)
    else:
        response_text, response_total, response_clipped = "", 0, False
        status = None
        response_headers = {}

    row = {
        "id": flow.id,
        "at": time.time(),
        "method": flow.request.method,
        "url": flow.request.pretty_url,
        "host": flow.request.pretty_host,
        "status": status,
        "error": str(flow.error) if flow.error else None,
        "request_headers": dict(flow.request.headers),
        "request_body": request_text,
        "request_body_bytes": request_total,
        "request_body_clipped": request_clipped,
        "response_headers": response_headers,
        "response_body": response_text,
        "response_body_bytes": response_total,
        "response_body_clipped": response_clipped,
        "blocked": "x-forge-blocked" in response_headers,
    }
    try:
        with open(CAPTURE, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(row) + "\n")
    except Exception as error:  # noqa: BLE001 - a capture write must never kill the proxy
        ctx.log.warn(f"forge: could not write capture row: {error}")
