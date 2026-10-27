"""Tiny WebKit remote-inspector client for the off-screen desktop harness.

Usage:  python3 inspector.py PORT file.js            evaluate a JS file (promises are awaited)
        python3 inspector.py PORT -e 'expression'    evaluate an expression
Library: `Inspector(port).eval(expr)` / `.send(method, params)` for raw protocol calls
(Timeline / ScriptProfiler / Heap domains).
"""
import json
import sys

import websocket


class Inspector:
    def __init__(self, port, timeout=120):
        self.ws = websocket.create_connection(f"ws://127.0.0.1:{port}/socket/1/1/WebPage", timeout=timeout)
        self.mid = 100
        self.events = []
        self.ws.send(json.dumps({"id": 1, "method": "Target.setPauseOnStart", "params": {"pauseOnStart": False}}))
        self.target = None
        while self.target is None:
            m = json.loads(self.ws.recv())
            if m.get("method") == "Target.targetCreated":
                self.target = m["params"]["targetInfo"]["targetId"]

    def send(self, method, params=None):
        self.mid += 1
        mid = self.mid
        inner = {"id": mid, "method": method, "params": params or {}}
        self.ws.send(json.dumps({"id": mid + 100000, "method": "Target.sendMessageToTarget",
                                 "params": {"targetId": self.target, "message": json.dumps(inner)}}))
        while True:
            m = json.loads(self.ws.recv())
            if m.get("method") == "Target.dispatchMessageFromTarget":
                r = json.loads(m["params"]["message"])
                if r.get("id") == mid:
                    return r
                self.events.append(r)

    def eval_async(self, expr, timeout=120):
        """WebKit's Runtime.evaluate ignores awaitPromise: park the result on window and poll."""
        import time
        self.eval("window.__r=undefined;window.__e=undefined;Promise.resolve(%s).then(v=>{window.__r=JSON.stringify(v===undefined?null:v)},e=>{window.__e=String(e&&e.stack||e)});0" % expr)
        end = time.time() + timeout
        while time.time() < end:
            v = self.eval("window.__e?'E'+window.__e:window.__r")
            if v is not None:
                if v.startswith("E") and v[1:2] != '"':
                    raise RuntimeError(v[:800])
                return json.loads(v)
            time.sleep(0.05)
        raise TimeoutError(expr[:80])

    def eval(self, expr):
        r = self.send("Runtime.evaluate", {"expression": expr, "returnByValue": True, "awaitPromise": True})
        res = r.get("result", {})
        if res.get("wasThrown"):
            raise RuntimeError(json.dumps(res)[:800])
        return res.get("result", {}).get("value")


if __name__ == "__main__":
    port = sys.argv[1]
    arg = sys.argv[2]
    src = sys.argv[3] if arg == "-e" else open(arg).read()
    i = Inspector(port)
    out = i.eval_async(src) if src.lstrip().startswith("new Promise") else i.eval(src)
    print(out if isinstance(out, str) else json.dumps(out))
