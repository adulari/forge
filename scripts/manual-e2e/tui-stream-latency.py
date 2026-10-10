#!/usr/bin/env python3
"""Measure TUI responsiveness while a long reply streams.

Drives `forge chat --mock --resume <session>` in a private tmux server (`tmux -L streamperf`, never
the default socket), streams the mock provider's long answer (`mock:long` paced, `mock:burst`
unpaced; size and pacing via FORGE_MOCK_LONG_TOKENS / FORGE_MOCK_TOKEN_DELAY_US) into a session
that already has a big transcript, and types while it streams. Reports the external keystroke-echo
latency (send-keys until the char shows in capture-pane) and prints the in-process histogram that
FORGE_TUI_PERF writes (draw / render / echo / iter / apply, microseconds).

Usage: BIG_DB=<copy of a store with a big session> RESUME=<session id> \
       python3 tui-stream-latency.py <forge-binary> <label> [mock:long|mock:burst] [tokens] [delay_us]
Only ever point BIG_DB at a COPY (sqlite3 live.db ".backup copy.db"): the run writes to it.
"""
import json, os, subprocess, sys, time, shutil

forge, label = sys.argv[1], sys.argv[2]
mode = sys.argv[3] if len(sys.argv) > 3 else "mock:long"
tokens = sys.argv[4] if len(sys.argv) > 4 else "20000"
delay_us = sys.argv[5] if len(sys.argv) > 5 else "3000"
resume = os.environ["RESUME"]
work = os.path.join(os.environ.get("WORK", "/tmp/forge-stream-latency"), f"run-{label}")
shutil.rmtree(work, ignore_errors=True)
os.makedirs(work + "/home", exist_ok=True)
db = work + "/forge.db"
shutil.copy(os.environ["BIG_DB"], db)
perf = work + "/perf.json"
sock = "streamperf"
T = ["tmux", "-L", sock]

def tm(*a, **k):
    return subprocess.run(T + list(a), capture_output=True, text=True, **k)

def screen():
    return tm("capture-pane", "-p", "-t", "sp").stdout

env = (f"HOME={work}/home XDG_CONFIG_HOME={work}/home/.config XDG_DATA_HOME={work}/home/.local/share "
       f"XDG_CACHE_HOME={work}/home/.cache XDG_STATE_HOME={work}/home/.state "
       f"FORGE_DB={db} FORGE_TUI_PERF={perf} FORGE_MOCK_LONG_TOKENS={tokens} FORGE_MOCK_TOKEN_DELAY_US={delay_us}")
cmd = f"env {env} {forge} chat --mock --resume {resume}"
tm("kill-session", "-t", "sp")
r = tm("new-session", "-d", "-s", "sp", "-x", "200", "-y", "50", cmd)
print("start", r.returncode, r.stderr.strip(), flush=True)
t0 = time.time()
while time.time() - t0 < 120:
    s = screen()
    if "Ctrl+K" in s or "/ for" in s or len(s.strip()) > 200:
        break
    time.sleep(0.5)
print("ready after %.1fs" % (time.time() - t0), flush=True)
time.sleep(2)
t0 = time.time()
while "compacted context" in screen() and time.time() - t0 < 30:
    tm("send-keys", "-t", "sp", "Enter")
    time.sleep(1.0)
while "resumed session" not in screen() and time.time() - t0 < 60:
    time.sleep(0.5)
time.sleep(2)

open(perf.replace('.json', '.reset'), 'w').close()
time.sleep(0.6)
tm("send-keys", "-t", "sp", "-l", f"{mode} stream it")
tm("send-keys", "-t", "sp", "Enter")
t_stream = time.time()
time.sleep(float(os.environ.get("LEAD", "4.0")))

lat = []
marker = "qzjwvkxhgfyp"
typed = ""
deadline = time.time() + float(os.environ.get("MEASURE_SECS", "60"))
i = 0
while time.time() < deadline:
    ch = marker[i % len(marker)]
    i += 1
    if i % len(marker) == 0:
        # clear the input line to keep it short
        tm("send-keys", "-t", "sp", "C-u")
        typed = ""
        time.sleep(0.2)
        continue
    typed += ch
    t1 = time.perf_counter()
    tm("send-keys", "-t", "sp", "-l", ch)
    seen = False
    while time.perf_counter() - t1 < 3.0:
        if typed in screen():
            seen = True
            break
    dt = (time.perf_counter() - t1) * 1000
    lat.append(dt if seen else 3000.0)
    time.sleep(0.15)
    streaming = "streaming" in screen() or "responding" in screen().lower()

lat.sort()
def pct(p):
    return lat[min(len(lat) - 1, max(0, -(-len(lat) * p // 100) - 1))]
print(json.dumps({"label": label, "samples": len(lat), "echo_ms_p50": round(pct(50), 1),
                  "echo_ms_p95": round(pct(95), 1), "echo_ms_max": round(lat[-1], 1),
                  "echo_ms_min": round(lat[0], 1)}), flush=True)
# still streaming?
print("streaming_still_going:", "streaming response" in screen() or "answer" in screen(), flush=True)
time.sleep(1.5)
try:
    print(open(perf).read(), flush=True)
except Exception as e:
    print("no perf file", e)
tm("send-keys", "-t", "sp", "C-c")
time.sleep(0.5)
tm("send-keys", "-t", "sp", "C-c")
time.sleep(1.0)
try:
    print("final perf:", open(perf).read(), flush=True)
except Exception:
    pass
tm("kill-session", "-t", "sp")
