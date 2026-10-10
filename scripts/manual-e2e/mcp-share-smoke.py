#!/usr/bin/env python3
"""Boot a real `forge serve` daemon against a local streamable-HTTP MCP server, create N sessions,
and count how many connections (initialize handshakes) the server saw.

Usage: python3 mcp-share-smoke.py <forge-binary> <label> <sessions>
Everything is isolated (HOME/XDG dirs, FORGE_DB, port 17431, `self_mcp = false`); no model is
called: the sessions are created and left idle. Before connection sharing N sessions meant N
handshakes and N open MCP sessions; with it, one.
"""
import json, os, shutil, subprocess, sys, threading, time, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

forge, label, n_sessions = sys.argv[1], sys.argv[2], int(sys.argv[3])
work = os.path.join(os.environ.get("WORK", "/tmp/forge-mcp-share-smoke"), f"mcp-{label}")
shutil.rmtree(work, ignore_errors=True)
os.makedirs(work + "/home/.config", exist_ok=True)
os.makedirs(work + "/proj/.forge", exist_ok=True)

stats = {"initialize": 0, "tools_list": 0, "sessions": set(), "posts": 0}
lock = threading.Lock()

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(405); self.send_header("Content-Length", "0"); self.end_headers()
    def do_DELETE(self):
        self.send_response(200); self.send_header("Content-Length", "0"); self.end_headers()
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        msg = json.loads(body or b"{}")
        with lock:
            stats["posts"] += 1
        if "id" not in msg:
            self.send_response(202); self.send_header("Content-Length", "0"); self.end_headers(); return
        method = msg.get("method")
        sid = self.headers.get("Mcp-Session-Id")
        extra = {}
        if method == "initialize":
            with lock:
                stats["initialize"] += 1
                sid = f"s{stats['initialize']}"
                stats["sessions"].add(sid)
            time.sleep(0.4)  # a slow handshake, like a remote server
            extra["Mcp-Session-Id"] = sid
            result = {"protocolVersion": msg["params"]["protocolVersion"], "capabilities": {"tools": {}},
                      "serverInfo": {"name": "fake-helm", "version": "1"}}
        elif method == "tools/list":
            with lock: stats["tools_list"] += 1
            result = {"tools": [{"name": "ping", "description": "ping", "inputSchema": {"type": "object"}}]}
        elif method == "resources/list":
            result = {"resources": []}
        elif method == "prompts/list":
            result = {"prompts": []}
        else:
            result = {}
        out = json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        for k, v in extra.items(): self.send_header(k, v)
        self.end_headers(); self.wfile.write(out)

srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
mcp_port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()

open(work + "/proj/.forge/mcp.toml", "w").write(
    f'[[servers]]\nname = "helm"\n[servers.transport]\ntype = "http"\nurl = "http://127.0.0.1:{mcp_port}/mcp"\n')
open(work + "/proj/.forge/config.toml", "w").write("self_mcp = false\n")

port = 17431
env = dict(os.environ, HOME=work + "/home", XDG_CONFIG_HOME=work + "/home/.config",
           XDG_DATA_HOME=work + "/home/.local/share", XDG_CACHE_HOME=work + "/home/.cache",
           XDG_STATE_HOME=work + "/home/.state", FORGE_DB=work + "/forge.db")
for d in ("home/.local/share", "home/.cache", "home/.state"):
    os.makedirs(os.path.join(work, d), exist_ok=True)
log = open(work + "/daemon.log", "w")
daemon = subprocess.Popen([forge, "serve", "--local", "--port", str(port)], cwd=work + "/proj", env=env,
                          stdout=log, stderr=subprocess.STDOUT)
try:
    base = None
    t0 = time.time()
    while time.time() - t0 < 90:
        txt = open(work + "/daemon.log").read()
        import re
        m = re.search(r"connect: (http://127\.0\.0\.1:%d/[0-9a-f]+)" % port, txt)
        if m:
            base = m.group(1); break
        if daemon.poll() is not None:
            print("daemon exited", daemon.returncode); print(txt[-2000:]); sys.exit(1)
        time.sleep(0.3)
    print("daemon up in %.1fs" % (time.time() - t0), flush=True)
    time.sleep(1.0)
    hdr = {"Content-Type": "application/json"}
    ids = []
    t1 = time.time()
    def make(i):
        req = urllib.request.Request(base + "/api/sessions", data=json.dumps(
            {"cwd": work + "/proj", "model": "ollama::none", "title": f"s{i}"}).encode(), headers=hdr, method="POST")
        try:
            r = urllib.request.urlopen(req, timeout=60)
            ids.append(json.loads(r.read()).get("id"))
        except Exception as e:
            ids.append(f"ERR {e}")
    threads = [threading.Thread(target=make, args=(i,)) for i in range(n_sessions)]
    [t.start() for t in threads]; [t.join() for t in threads]
    print("sessions created:", ids, "in %.1fs" % (time.time() - t1), flush=True)
    time.sleep(4.0)
    with lock:
        print(json.dumps({"label": label, "sessions": n_sessions, "mcp_initialize_handshakes": stats["initialize"],
                          "tools_list_calls": stats["tools_list"], "open_mcp_sessions": len(stats["sessions"]),
                          "posts": stats["posts"]}), flush=True)
finally:
    daemon.terminate()
    try: daemon.wait(10)
    except Exception: daemon.kill()
    srv.shutdown()
