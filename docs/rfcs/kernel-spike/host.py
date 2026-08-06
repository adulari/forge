# Spike host: plays Forge's session core. Policy: writes under ALLOW_DIR allowed, everything else
# denied; shell denied entirely. Measures startup, exec round-trip, state persistence across a
# kernel restart via snapshot/restore, and the deny path.
import json
import subprocess
import sys
import time
import os

ALLOW_DIR = os.path.abspath(sys.argv[1])


def start():
    t0 = time.perf_counter()
    p = subprocess.Popen(
        [sys.executable, os.path.join(os.path.dirname(__file__), "kernel.py")],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    return p, (time.perf_counter() - t0) * 1000


def rpc(p, msg):
    p.stdin.write(json.dumps(msg) + "\n")
    p.stdin.flush()
    while True:
        line = p.stdout.readline()
        m = json.loads(line)
        if m["op"] == "host_request":
            ok, result = policy(m["type"], m["payload"])
            p.stdin.write(json.dumps({"op": "host_response", "id": m["id"], "ok": ok, "result": result}) + "\n")
            p.stdin.flush()
            continue
        return m


def shutdown(p):
    # `shutdown` breaks the kernel's loop without replying — never wait for a response.
    p.stdin.write(json.dumps({"op": "shutdown"}) + "\n")
    p.stdin.flush()
    p.wait(timeout=5)


def policy(rtype, payload):
    if rtype == "fs.write":
        path = os.path.abspath(payload["path"])
        if path.startswith(ALLOW_DIR + os.sep):
            with open(path, "w") as f:
                f.write(payload["content"])
            return True, "written"
        return False, f"write outside allowed dir refused: {path}"
    if rtype == "fs.read":
        path = os.path.abspath(payload["path"])
        if path.startswith(ALLOW_DIR + os.sep):
            with open(path) as f:
                return True, f.read()
        return False, "read outside allowed dir refused"
    return False, f"{rtype} not permitted in this spike"


p, startup_ms = start()
t = time.perf_counter()
r = rpc(p, {"op": "exec", "code": "x = sum(range(1000000)); print('warm')"})
first_exec_ms = (time.perf_counter() - t) * 1000
assert r["error"] is None and "warm" in r["stdout"]

r = rpc(p, {"op": "exec", "code": "print(x)"})
assert r["stdout"].strip() == "499999500000", r
print(f"STATE_PERSISTS_ACROSS_CALLS ok  startup={startup_ms:.0f}ms first_exec={first_exec_ms:.0f}ms")

r = rpc(p, {"op": "exec", "code": f"forge.write_file({ALLOW_DIR!r} + '/ok.txt', 'hello')"})
assert r["error"] is None, r
r = rpc(p, {"op": "exec", "code": f"print(forge.read_file({ALLOW_DIR!r} + '/ok.txt'))"})
assert r["stdout"].strip() == "hello", r
print("BROKERED_WRITE_AND_READ ok")

r = rpc(p, {"op": "exec", "code": "forge.write_file('/tmp/forbidden-spike.txt', 'nope')"})
assert r["error"] is not None and "denied" in r["error"], r
assert not os.path.exists("/tmp/forbidden-spike.txt")
print(f"DENIED_WRITE_FAILS_INSIDE_SCRIPT ok  error={r['error']!r}")

r = rpc(p, {"op": "exec", "code": "forge.shell('rm -rf /')"})
assert r["error"] is not None and "denied" in r["error"], r
print("DENIED_SHELL ok")

r = rpc(p, {"op": "exec", "code": "try:\n    forge.write_file('/etc/nope', 'x')\nexcept PermissionError as e:\n    recovered = 'script continued after deny'\nprint(recovered)"})
assert "continued after deny" in r["stdout"], r
print("SCRIPT_RECOVERS_FROM_DENY ok")

snap = rpc(p, {"op": "snapshot"})
shutdown(p)

p2, restart_ms = start()
rpc(p2, {"op": "restore", "blob": snap["blob"]})
r = rpc(p2, {"op": "exec", "code": "print(x)"})
assert r["stdout"].strip() == "499999500000", r
print(f"STATE_SURVIVES_KERNEL_RESTART_VIA_SNAPSHOT ok  restart={restart_ms:.0f}ms  blob={len(snap['blob'])}B")

r = rpc(p2, {"op": "exec", "code": "open('/tmp/forbidden-direct.txt','w').write('leak')"})
leak = os.path.exists("/tmp/forbidden-direct.txt")
print(f"DIRECT_OPEN_BYPASSES_BROKER: {leak}  (finding: OS-level sandbox still required for the kernel process)")
if leak:
    os.remove("/tmp/forbidden-direct.txt")
shutdown(p2)
print("SPIKE_COMPLETE")
