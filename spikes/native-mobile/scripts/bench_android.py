#!/usr/bin/env python3
"""Android benchmark driver: native spike vs the Expo RN release APK on one emulator.

Needs: adb on PATH + ANDROID_SERIAL, the fixture server running (fixture/server.py, log to
--fixture-log) and both APKs installed. Writes JSON to --out.
"""
import argparse, json, re, statistics, subprocess, sys, time, urllib.request

ap = argparse.ArgumentParser()
ap.add_argument("--fixture-log", required=True)
ap.add_argument("--fixture-port", default="7431")
ap.add_argument("--token", required=True)
ap.add_argument("--session", required=True)
ap.add_argument("--runs", type=int, default=7)
ap.add_argument("--out", required=True)
ap.add_argument("--apps", default="native,rn")
ap.add_argument("--dex", choices=["reset", "speed"], default="reset", help="ART state: as-installed (JIT) or fully AOT-compiled")
a = ap.parse_args()

NATIVE = "dev.adulari.forge.spike"
RN = "dev.adulari.forge"
BASE = f"http://10.0.2.2:{a.fixture_port}/{a.token}"

def sh(cmd, check=False):
    return subprocess.run(["adb", "shell", cmd], capture_output=True, text=True).stdout

def launch(app, target):
    cmd = {
        (NATIVE, "fleet"): f"am start -W -n {NATIVE}/.MainActivity --es base {BASE}",
        (NATIVE, "session"): f"am start -W -n {NATIVE}/.MainActivity --es base {BASE} --es session {a.session}",
        (RN, "fleet"): f"am start -W -n {RN}/.MainActivity",
        (RN, "session"): f"am start -W -a android.intent.action.VIEW -d forge://session/{a.session} {RN}",
    }[(app, target)]
    t0 = int(time.time() * 1000)
    out = sh(cmd)
    m = re.search(r"TotalTime: (\d+)", out)
    return t0, int(m.group(1)) if m else None

def first_req(kind, after_ms, timeout=20):
    end = time.time() + timeout
    while time.time() < end:
        for line in open(a.fixture_log):
            p = line.split()
            if len(p) == 3 and p[0] == "REQ" and p[2] == kind and int(p[1]) >= after_ms:
                return int(p[1]) - after_ms
        time.sleep(0.1)
    return None

def pid(app):
    return sh(f"pidof {app}").strip().split()[0] if sh(f"pidof {app}").strip() else None

def meminfo(app):
    out = sh(f"dumpsys meminfo {app}")
    def row(name):
        m = re.search(rf"^\s*{name}:?\s+(\d+)", out, re.M)
        return int(m.group(1)) // 1 if m else None
    tot = re.search(r"TOTAL PSS:\s+(\d+)", out)
    return {"total_pss_kb": int(tot.group(1)) if tot else None, "java_heap_kb": row("Java Heap"),
            "native_heap_kb": row("Native Heap"), "graphics_kb": row("Graphics"), "code_kb": row("Code")}

def gfx_reset(app): sh(f"dumpsys gfxinfo {app} reset")

def gfx(app):
    out = sh(f"dumpsys gfxinfo {app}")
    g = lambda pat: (lambda m: float(m.group(1)) if m else None)(re.search(pat, out))
    return {"frames": g(r"Total frames rendered: (\d+)"), "janky": g(r"Janky frames: (\d+)"),
            "janky_pct": g(r"Janky frames: \d+ \(([\d.]+)%\)"), "p50_ms": g(r"50th percentile: (\d+)ms"),
            "p90_ms": g(r"90th percentile: (\d+)ms"), "p95_ms": g(r"95th percentile: (\d+)ms"),
            "p99_ms": g(r"99th percentile: (\d+)ms"), "missed_vsync": g(r"Number Missed Vsync: (\d+)"),
            "slow_ui": g(r"Number Slow UI thread: (\d+)"), "slow_draw": g(r"Number Slow issue draw commands: (\d+)")}

def cpu_ticks(p):
    f = sh(f"cat /proc/{p}/stat").split(")")[-1].split()
    return int(f[11]) + int(f[12])  # utime + stime (fields 14,15)

def swipes(n_down=40, n_up=10):
    script = "for i in $(seq %d); do input swipe 540 700 540 1900 140; sleep 0.25; done; for i in $(seq %d); do input swipe 540 1900 540 700 140; sleep 0.25; done" % (n_down, n_up)
    sh(script)

def stop(app): sh(f"am force-stop {app}")

def stream(tokens):
    urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{a.fixture_port}/{a.token}/bench/stream?session={a.session}&tokens={tokens}")).read()

def med(xs):
    xs = [x for x in xs if x is not None]
    return {"median": statistics.median(xs), "min": min(xs), "max": max(xs), "all": xs} if xs else None

res = {}
sh("input keyevent KEYCODE_WAKEUP; svc power stayon true")
for pkg in (NATIVE, RN):
    sh(f"cmd package compile --reset {pkg}" if a.dex == "reset" else f"cmd package compile -m speed -f {pkg}")
res["_meta"] = {"dex": a.dex, "runs": a.runs}
for app, label in [x for x in ((NATIVE, "native"), (RN, "rn")) if x[1] in a.apps.split(",")]:
    r = {}
    for target in ("fleet", "session"):
        tt, first_data = [], []
        for _ in range(a.runs):
            stop(NATIVE); stop(RN); sh("input keyevent KEYCODE_HOME"); time.sleep(2.5)
            t0, total = launch(app, target)
            tt.append(total)
            first_data.append(first_req("/api/history" if target == "session" else "/api/sessions", t0))
            time.sleep(1)
        r[f"cold_{target}_totaltime_ms"] = med(tt)
        r[f"cold_{target}_first_data_request_ms"] = med(first_data)
    # memory: fleet idle, then session open (+10s settle), then after scrolling the whole transcript
    stop(NATIVE); stop(RN); time.sleep(2)
    launch(app, "fleet"); time.sleep(10)
    r["pss_fleet_idle"] = meminfo(app)
    stop(app); time.sleep(2)
    launch(app, "session"); time.sleep(10)
    r["pss_session_open"] = meminfo(app)
    # scroll jank over the transcript
    gfx_reset(app); swipes(); time.sleep(1)
    r["scroll_gfx"] = gfx(app)
    r["pss_after_scroll"] = meminfo(app)
    # streaming: idle CPU baseline then ~50 tok/s for 12s (600 tokens)
    p = pid(app)
    c0 = cpu_ticks(p); time.sleep(12); c1 = cpu_ticks(p)
    r["cpu_idle_pct_of_one_core"] = (c1 - c0) / 12.0
    gfx_reset(app)
    c0 = cpu_ticks(p); pss0 = meminfo(app)["total_pss_kb"]
    stream(600); time.sleep(14.5); c1 = cpu_ticks(p)
    r["stream_cpu_pct_of_one_core"] = (c1 - c0) / 14.5
    r["stream_gfx"] = gfx(app)
    r["pss_after_stream"] = meminfo(app)
    r["pss_stream_delta_kb"] = r["pss_after_stream"]["total_pss_kb"] - pss0
    res[label] = r
    stop(app)
json.dump(res, open(a.out, "w"), indent=2)
print(json.dumps(res, indent=2))
