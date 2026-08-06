# Spike: persistent Python control process for Forge's kernel RFC.
# Protocol (line-JSON on stdio):
#   host -> kernel: {"op":"exec","code": "..."}          run code in the persistent namespace
#   kernel -> host: {"op":"host_request","id":N,"type":"fs.write"|"fs.read"|"shell","payload":{...}}
#   host -> kernel: {"op":"host_response","id":N,"ok":bool,"result":...}
#   kernel -> host: {"op":"exec_done","stdout": "...", "error": null|"..."}
# The kernel has NO direct ambient authority: forge.* builtins are the only IO surface, and each
# one round-trips a host_request. Direct open()/os is deliberately left available in this spike to
# measure what an OS-sandbox layer must still block (finding recorded, not solved here).
import json
import sys
import io
import contextlib
import pickle

_req_id = 0


# Captured before exec ever runs: user code executes under redirect_stdout, so the protocol must
# write to the REAL stdout or every host_request would land in the captured buffer and deadlock.
_PROTO_OUT = sys.stdout


def _host_request(rtype, payload):
    global _req_id
    _req_id += 1
    _PROTO_OUT.write(
        json.dumps({"op": "host_request", "id": _req_id, "type": rtype, "payload": payload}) + "\n"
    )
    _PROTO_OUT.flush()
    line = sys.stdin.readline()
    resp = json.loads(line)
    assert resp["op"] == "host_response" and resp["id"] == _req_id
    if not resp["ok"]:
        raise PermissionError(f"host denied {rtype}: {resp.get('result')}")
    return resp["result"]


class Forge:
    def write_file(self, path, content):
        return _host_request("fs.write", {"path": path, "content": content})

    def read_file(self, path):
        return _host_request("fs.read", {"path": path})

    def shell(self, cmd):
        return _host_request("shell", {"cmd": cmd})


NAMESPACE = {"forge": Forge()}


def snapshot():
    keep = {}
    for k, v in NAMESPACE.items():
        if k == "forge" or k.startswith("__"):
            continue
        try:
            keep[k] = pickle.dumps(v)
        except Exception:
            pass
    return pickle.dumps(keep)


def restore(blob):
    keep = pickle.loads(blob)
    for k, v in keep.items():
        try:
            NAMESPACE[k] = pickle.loads(v)
        except Exception:
            pass


# NOT `for line in sys.stdin`: the file iterator read-ahead-buffers, which would swallow the
# host_response line that _host_request() later blocks on. readline() reads exactly one line.
for line in iter(sys.stdin.readline, ""):
    msg = json.loads(line)
    if msg["op"] == "exec":
        out = io.StringIO()
        err = None
        try:
            with contextlib.redirect_stdout(out):
                exec(msg["code"], NAMESPACE)
        except Exception as e:
            err = f"{type(e).__name__}: {e}"
        print(json.dumps({"op": "exec_done", "stdout": out.getvalue(), "error": err}), flush=True)
    elif msg["op"] == "snapshot":
        import base64

        print(json.dumps({"op": "snapshot_done", "blob": base64.b64encode(snapshot()).decode()}), flush=True)
    elif msg["op"] == "restore":
        import base64

        restore(base64.b64decode(msg["blob"]))
        print(json.dumps({"op": "restore_done"}), flush=True)
    elif msg["op"] == "shutdown":
        break
